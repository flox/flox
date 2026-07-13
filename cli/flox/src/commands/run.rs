//! `flox run` — resolve a catalog package and exec an executable from it.
//!
//! bpaf cannot implement POSIX stop-at-first-positional parsing (validated
//! against bpaf 0.9.24 vendored source, `args.rs:372-392`):
//!
//! 1. bpaf consumes the first `--` before `any()` catchalls see it, losing
//!    a distinction `flox run` needs.
//! 2. bpaf's flag recognition is order-independent, so in
//!    `flox run curl -p curl` it would wrongly claim `-p curl` for flox —
//!    POSIX rules say it belongs to `curl`.
//!
//! So `flox` splits argv itself; bpaf only dispatches the `run` subcommand.
//! `Run._raw_args` is an unconditional catchall so bpaf never intercepts
//! flags that belong to the invoked command. `handle()` re-reads raw process
//! arguments with `std::env::args_os()`.

use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStringExt as _;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::Result;
use bpaf::Bpaf;
use flox_config::{Config, ReadWriteError};
use flox_manifest::raw::{CatalogPackage, RawManifestError};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::providers::buildenv::{
    BuildEnvError,
    build_catalog_pkg_from_source,
    copy_from_custom_catalog_locations,
    materialise_with_retry,
    substitute_store_paths,
};
use flox_rust_sdk::providers::nix_auth::{AuthProvider, NixAuth};
use floxhub_client::{
    CatalogClientTrait,
    MessageLevel,
    PackageDescriptor,
    PackageGroup,
    PackageSystem,
    ResolutionMessage,
};
use indoc::indoc;
use thiserror::Error;
use toml_edit::Key;
use tracing::{debug, info_span};

use crate::commands::general::update_config_with_query;
use crate::subcommand_metric;
use crate::utils::dialog::{Dialog, Select};
use crate::utils::message;

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

/// Errors specific to `flox run`.
#[derive(Debug, Error)]
pub enum RunError {
    /// No command was given after parsing all flags.
    #[error(
        "No command specified.\n\
         Run 'flox run [--package <PACKAGE>] <COMMAND> [ARGS...]'."
    )]
    NoExecutable,

    /// `-p`/`--package` flag appeared without a value.
    #[error(
        "Missing value for '{0}'.\n\
         Use '--package <PACKAGE>' to specify the package that provides the command."
    )]
    MissingPackageValue(String),

    /// The value passed to `-p`/`--package` was not valid UTF-8.
    #[error("Package specs must be valid UTF-8.")]
    PackageSpecNotUtf8,

    /// `CatalogPackage::from_str` failed.
    #[error(
        "Invalid package '{0}'.\n\
         Use 'flox search' to find available packages."
    )]
    InvalidPackageSpec(String, #[source] RawManifestError),

    /// Package spec uses unsupported syntax (`^`).
    #[error(
        "Unsupported package '{0}'.\n\
         'flox run' accepts a package name with an optional version constraint ('@'); \
         output selectors ('^') are not supported."
    )]
    UnsupportedPackageSpec(String),

    /// An unrecognised flag appeared before the command name.
    #[error(
        "Unknown option '{0}'.\n\
         Use '--' before the command name if it starts with '-'."
    )]
    UnknownFlag(String),

    /// The command name is not valid UTF-8, so it cannot be looked up.
    #[error(
        "Command names must be valid UTF-8 to look up packages.\n\
         Use '--package <PACKAGE>' to specify the package explicitly."
    )]
    CommandNotUtf8,

    /// The binary-to-package lookup returned no candidates.
    #[error(
        "No packages found that provide '{0}'.\n\
         Use 'flox run --package <PACKAGE> {0}' to specify the package directly."
    )]
    BinaryNotFound(String),

    /// Several packages provide the binary and no preference is saved,
    /// but there is no terminal to prompt on.
    #[error(
        "Multiple packages provide '{binary}' and no preference is saved.\n\
         Packages with this binary: {package_list}\n\
         Use 'flox run --package <PACKAGE> {binary}' to specify a package."
    )]
    AmbiguousBinary {
        binary: String,
        package_list: String,
    },

    /// `--reselect` needs a terminal to re-prompt on.
    #[error(
        "'--reselect' requires an interactive terminal.\n\
         Use 'flox run --package <PACKAGE> {0}' to specify a package."
    )]
    ReselectRequiresTerminal(String),

    /// Transport/network failure during the binary-to-package lookup.
    #[error(
        "Failed to look up packages that provide '{0}'.\n\
         Check your network connection and try again."
    )]
    LookupFailed(String),

    /// Package was not found in the Flox Catalog.
    #[error(
        "Package '{0}' was not found in the Flox Catalog.\n\
         Use 'flox search {0}' to find available packages."
    )]
    PackageNotFound(String),

    /// Package exists but is not available for the current system.
    #[error("Package '{0}' is not available for system '{1}'.")]
    PackageUnavailableOnSystem(String, String),

    /// The catalog returned an error-level resolution message not otherwise classified.
    #[error(
        "Failed to resolve package '{0}'.\n\
         {1}"
    )]
    ResolutionMessage(String, String),

    /// Transport/network failure during catalog resolve.
    #[error(
        "Failed to resolve package '{0}'.\n\
         Check your network connection and try again."
    )]
    CatalogError(String),

    /// The resolved package has no store paths for this system.
    #[error("Package '{0}' has no store paths to download for this system.")]
    NoStorePaths(String),

    /// Creating the GC-root cache directory failed.
    #[error("Failed to prepare the cache directory for '{0}'.")]
    CreateGcRootDir(String, #[source] std::io::Error),

    /// The `nix build` invocation for building from source failed.
    #[error(
        "Failed to build '{0}' from source.\n\
         Use 'flox install {0}' to add it to a persistent environment."
    )]
    BuildFailed(String, #[source] BuildEnvError),

    /// The requested executable was not found in `bin/` or `sbin/` of any output.
    #[error(
        "Command '{executable}' was not found in package '{package}'.\n\
         The package may provide the command under a different name."
    )]
    ExecutableNotFound { executable: String, package: String },

    /// The final `exec` syscall returned (rare — permissions or missing binary).
    #[error("Failed to run '{0}'.")]
    ExecFailed(String, #[source] std::io::Error),
}

// ---------------------------------------------------------------------------
// Parsed argument types
// ---------------------------------------------------------------------------

/// Outcome of the `parse_run_args` state machine.
#[derive(Debug, Clone, PartialEq)]
pub enum ParsedArgs {
    /// `-h`/`--help` was seen before the first positional or `--`.
    Help,
    /// A fully-specified run invocation.
    Run(RunArgs),
}

/// Validated arguments produced by the POSIX state machine.
#[derive(Debug, Clone, PartialEq)]
pub struct RunArgs {
    /// Package spec from `-p`/`--package`.
    /// When absent, the command name is looked up in the catalog's
    /// binary-to-package index.
    pub package: Option<String>,
    /// Clear the saved package preference for the command and choose again.
    pub reselect: bool,
    /// Command name (first positional argument).
    pub executable: OsString,
    /// Remaining arguments forwarded verbatim to the command.
    pub args: Vec<OsString>,
}

// ---------------------------------------------------------------------------
// bpaf registration struct
// ---------------------------------------------------------------------------

/// Run a command from a Flox Catalog package.
#[derive(Bpaf, Clone, Debug)]
pub struct Run {
    // Unconditional catchall: bpaf dispatches the subcommand but never
    // intercepts any flag, including -h/--help. handle() re-reads argv via
    // args_os() and delegates to parse_run_args().
    #[bpaf(any("ARGS", Some), many)]
    _raw_args: Vec<String>,
}

impl Run {
    /// Entry point: parse args with POSIX stop-at-first-positional semantics,
    /// then resolve, download, and exec.
    pub async fn handle(self, config: Config, flox: Flox) -> Result<()> {
        subcommand_metric!("run");

        // Re-read raw OS args. bpaf has already consumed the first `--`, so
        // we cannot rely on self._raw_args for correct passthrough semantics.
        // Locating the first "run" token is safe: the only options before a
        // subcommand are boolean flags (-v, --debug), so the first "run" token
        // is always the subcommand keyword.
        let all_args: Vec<OsString> = std::env::args_os().collect();
        let run_idx = all_args
            .iter()
            .position(|a| a == "run")
            .unwrap_or(all_args.len());
        let after_run: Vec<OsString> = all_args[run_idx + 1..].to_vec();

        let parsed = parse_run_args(after_run).map_err(anyhow::Error::from)?;

        match parsed {
            ParsedArgs::Help => {
                print_help();
                Ok(())
            },
            ParsedArgs::Run(run_args) => exec_run(run_args, &config, &flox).await,
        }
    }
}

// ---------------------------------------------------------------------------
// Arg pre-processor (POSIX stop-at-first-positional state machine)
// ---------------------------------------------------------------------------

/// Parse the arguments that follow `flox run` using POSIX stop-at-first-positional
/// semantics.
///
/// Flag rules (before the first positional or `--`):
/// - `-h` / `--help` → `ParsedArgs::Help`
/// - `-p` / `--package` (space form only) → consume next arg as package spec
/// - `-p=…` / `--package=…` / bundled forms → `UnknownFlag`
/// - `--reselect` → clear the saved package preference and choose again
/// - `--` → force positional mode; next arg is the command even if it starts with `-`
/// - any other `"-…"` → `UnknownFlag`
///
/// After the command name, a single `--` immediately following it is the
/// command/arguments separator and is dropped; everything else is forwarded
/// verbatim, including any later literal `--`.
pub fn parse_run_args(args: Vec<OsString>) -> Result<ParsedArgs, RunError> {
    let mut package: Option<String> = None;
    let mut reselect = false;
    let mut executable: Option<OsString> = None;
    let mut passthrough: Vec<OsString> = Vec::new();

    let mut iter = args.into_iter();

    // Step 1: scan flags until we see `--` or the first positional.
    'flags: loop {
        let Some(arg) = iter.next() else {
            break 'flags;
        };

        match arg.to_str() {
            Some("--") => {
                // Force positional mode: the next arg is the command even if
                // it starts with `-`. Everything after it is passthrough.
                if let Some(cmd) = iter.next() {
                    executable = Some(cmd);
                }
                extend_passthrough(&mut passthrough, iter);
                break 'flags;
            },
            Some("-h") | Some("--help") => {
                return Ok(ParsedArgs::Help);
            },
            Some("-p") | Some("--package") => {
                let value_os = iter.next().ok_or_else(|| {
                    RunError::MissingPackageValue(arg.to_string_lossy().into_owned())
                })?;
                let value = value_os
                    .into_string()
                    .map_err(|_| RunError::PackageSpecNotUtf8)?;
                package = Some(value);
            },
            Some("--reselect") => {
                reselect = true;
            },
            Some(s) if s.starts_with('-') => {
                return Err(RunError::UnknownFlag(s.to_owned()));
            },
            _ => {
                // First non-flag positional is the command name; everything
                // after it is passthrough.
                executable = Some(arg);
                extend_passthrough(&mut passthrough, iter);
                break 'flags;
            },
        }
    }

    let executable = executable.ok_or(RunError::NoExecutable)?;

    Ok(ParsedArgs::Run(RunArgs {
        package,
        reselect,
        executable,
        args: passthrough,
    }))
}

/// Forward the arguments that follow the command name.
///
/// A single `--` immediately after the command name is the
/// command/arguments separator (`flox run curl -- -sL <URL>`) and is
/// dropped; everything else is forwarded verbatim, including any later
/// literal `--`.
fn extend_passthrough(passthrough: &mut Vec<OsString>, mut iter: impl Iterator<Item = OsString>) {
    match iter.next() {
        Some(first) if first == "--" => {},
        Some(first) => passthrough.push(first),
        None => {},
    }
    passthrough.extend(iter);
}

// ---------------------------------------------------------------------------
// Package spec validation
// ---------------------------------------------------------------------------

/// Reject package specs that use unsupported syntax.
///
/// Accepts an attr-path (`cowsay`, `python3Packages.requests`), optionally
/// with a version constraint (`curl@8.0`), or a custom catalog package
/// (`mycatalog/vim`). Output selectors (`^`) are not supported.
pub fn validate_plain_package(pkg: &CatalogPackage, raw: &str) -> Result<(), RunError> {
    if pkg.outputs.is_some() {
        return Err(RunError::UnsupportedPackageSpec(raw.to_string()));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Binary-to-package selection
// ---------------------------------------------------------------------------

/// A package that provides a requested binary, as returned by the
/// binary-to-package lookup.
#[derive(Clone, Debug, PartialEq)]
pub struct PackageCandidate {
    /// Full attribute path (e.g. `binutils`).
    pub attr_path: String,
    /// Package name (e.g. `binutils`).
    pub pname: String,
    /// Human-readable description.
    pub description: Option<String>,
    /// Package version.
    pub version: Option<String>,
}

/// Menu row rendering for the disambiguation prompt.
impl std::fmt::Display for PackageCandidate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.attr_path)?;
        if self.pname != self.attr_path {
            write!(f, " ({})", self.pname)?;
        } else if let Some(ref version) = self.version {
            write!(f, " ({})", version)?;
        }
        if let Some(ref description) = self.description {
            let truncated = if description.len() > 60 {
                format!("{description:.57}...")
            } else {
                description.clone()
            };
            write!(f, " — {truncated}")?;
        }
        Ok(())
    }
}

/// How the package for a run was chosen.
///
/// Choices the user made explicitly (`--package`, the disambiguation prompt)
/// are saved as preferences; choices derived deterministically from the
/// lookup are not.
#[derive(Clone, Debug, PartialEq)]
enum PackageSelection {
    /// `-p`/`--package` was given on the command line.
    Explicit(String),
    /// A saved preference from the config was used silently.
    Saved(String),
    /// The lookup returned exactly one candidate.
    OnlyCandidate(String),
    /// Several candidates, but one is named exactly like the binary.
    ExactNameMatch(String),
    /// The user chose from the disambiguation prompt.
    Prompted(String),
}

impl PackageSelection {
    fn pkg_spec(&self) -> &str {
        match self {
            PackageSelection::Explicit(spec)
            | PackageSelection::Saved(spec)
            | PackageSelection::OnlyCandidate(spec)
            | PackageSelection::ExactNameMatch(spec)
            | PackageSelection::Prompted(spec) => spec,
        }
    }

    /// Only explicit user choices are persisted as preferences.
    fn should_save(&self) -> bool {
        matches!(
            self,
            PackageSelection::Explicit(_) | PackageSelection::Prompted(_)
        )
    }
}

/// Choose the package that provides the requested command.
///
/// Selection order:
/// 1. `-p`/`--package` given → use it (and save it as a preference later).
/// 2. `--reselect` → clear the saved preference and fall through to lookup.
/// 3. Saved preference → use it silently.
/// 4. Catalog binary-to-package lookup:
///    - no candidates → error
///    - one candidate → use it silently
///    - several, one named exactly like the command → use it silently
///    - several, terminal available → interactive prompt (choice saved)
///    - several, no terminal → error listing the candidates
async fn select_package(
    run_args: &RunArgs,
    config: &Config,
    flox: &Flox,
) -> Result<PackageSelection> {
    if let Some(pkg_spec) = &run_args.package {
        return Ok(PackageSelection::Explicit(pkg_spec.clone()));
    }

    let Some(binary) = run_args.executable.to_str() else {
        return Err(RunError::CommandNotUtf8.into());
    };

    if run_args.reselect {
        if !Dialog::can_prompt() {
            return Err(RunError::ReselectRequiresTerminal(binary.to_string()).into());
        }
        clear_preference(&flox.config_dir, binary);
    } else if let Some(saved) = config.flox.binary_preferences.get(binary) {
        debug!(binary, package = %saved, "using saved package preference");
        return Ok(PackageSelection::Saved(saved.clone()));
    }

    let candidates = lookup_binary_candidates(binary, flox).await?;

    if candidates.is_empty() {
        return Err(RunError::BinaryNotFound(binary.to_string()).into());
    }

    if candidates.len() == 1 {
        let candidate = &candidates[0];
        if candidate.attr_path != binary {
            message::plain(format!(
                "Running '{binary}' from package '{}'.",
                candidate.attr_path
            ));
        }
        return Ok(PackageSelection::OnlyCandidate(candidate.attr_path.clone()));
    }

    // Several candidates: a package named exactly like the binary wins
    // silently; otherwise prompt (or fail without a terminal).
    if let Some(exact) = candidates.iter().find(|c| c.attr_path == binary) {
        debug!(
            binary,
            count = candidates.len(),
            "several candidates, exact name match wins"
        );
        return Ok(PackageSelection::ExactNameMatch(exact.attr_path.clone()));
    }

    if !Dialog::can_prompt() {
        let package_list = candidates
            .iter()
            .map(|c| c.attr_path.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(RunError::AmbiguousBinary {
            binary: binary.to_string(),
            package_list,
        }
        .into());
    }

    let chosen = choose_package_interactive(binary, &candidates)?;
    Ok(PackageSelection::Prompted(chosen.attr_path))
}

/// Look up the packages that provide `binary` via the catalog.
///
/// Tries the dedicated binary-to-package index first; if the endpoint is
/// unavailable (e.g. an older FloxHub), falls back to a search-based
/// heuristic that keeps exact name matches only.
async fn lookup_binary_candidates(
    binary: &str,
    flox: &Flox,
) -> Result<Vec<PackageCandidate>, RunError> {
    let system: PackageSystem = flox.system.parse().map_err(|_| {
        RunError::PackageUnavailableOnSystem(binary.to_string(), flox.system.clone())
    })?;

    match flox.floxhub_client.packages_by_binary(binary, system).await {
        Ok(page) => {
            debug!(
                binary,
                count = page.results.len(),
                "found candidates via by-binary index"
            );
            let candidates = page
                .results
                .into_iter()
                .map(|pkg| PackageCandidate {
                    attr_path: pkg.attr_path,
                    pname: pkg.pname,
                    description: pkg.description,
                    version: Some(pkg.version),
                })
                .collect();
            Ok(dedupe_candidates(candidates))
        },
        Err(err) => {
            debug!(binary, error = %err, "by-binary index unavailable, falling back to search");
            let candidates = search_fallback_candidates(binary, system, flox).await?;
            Ok(dedupe_candidates(candidates))
        },
    }
}

/// Search-based fallback for catalogs without the binary-to-package index.
///
/// Searches for the binary name and keeps only results whose `pname` or last
/// attr-path segment matches the binary exactly.
async fn search_fallback_candidates(
    binary: &str,
    system: PackageSystem,
    flox: &Flox,
) -> Result<Vec<PackageCandidate>, RunError> {
    let limit = std::num::NonZeroU8::new(20);
    let results = flox
        .floxhub_client
        .search(binary, system, limit)
        .await
        .map_err(|err| {
            debug!(binary, error = %err, "search fallback failed");
            RunError::LookupFailed(binary.to_string())
        })?;

    let candidates = results
        .results
        .into_iter()
        .filter(|pkg| {
            let last_segment = pkg.attr_path.rsplit('.').next().unwrap_or(&pkg.attr_path);
            last_segment == binary || pkg.pname == binary
        })
        .map(|pkg| PackageCandidate {
            attr_path: pkg.attr_path,
            pname: pkg.pname,
            description: pkg.description,
            version: pkg.version,
        })
        .collect();

    Ok(candidates)
}

/// Drop duplicate attr-paths, keeping the first (highest-ranked) occurrence.
///
/// The by-binary index returns one row per package version; the
/// disambiguation prompt should list each package once.
fn dedupe_candidates(candidates: Vec<PackageCandidate>) -> Vec<PackageCandidate> {
    let mut seen = std::collections::HashSet::new();
    candidates
        .into_iter()
        .filter(|c| seen.insert(c.attr_path.clone()))
        .collect()
}

/// Show the disambiguation prompt and return the chosen candidate.
///
/// Callers must check `Dialog::can_prompt()` first. The prompt renders on
/// stderr, so piped stdout stays clean.
fn choose_package_interactive(
    binary: &str,
    candidates: &[PackageCandidate],
) -> Result<PackageCandidate> {
    let message = format!("Multiple packages provide '{binary}'. Which would you like to use?");
    let (_, chosen) = Dialog {
        message: &message,
        help_message: Some("Use arrow keys to select, Enter to confirm"),
        typed: Select {
            options: candidates.to_vec(),
        },
    }
    .raw_prompt()?;

    Ok(chosen)
}

/// Persist a command → package preference in the user config.
///
/// Uses a pre-parsed key query so command names containing dots
/// (e.g. `python3.12`) stay a single TOML key.
fn save_preference(config_dir: &Path, binary: &str, pkg_spec: &str) -> Result<()> {
    let query = [Key::new("binary_preferences"), Key::new(binary)];
    update_config_with_query(config_dir, &query, Some(pkg_spec))
}

/// Remove a saved command → package preference from the user config.
///
/// A missing key is not an error: `--reselect` on a command that never had
/// a preference just falls through to the lookup. Other write failures are
/// logged and ignored — the subsequent prompt selection will overwrite the
/// entry anyway.
fn clear_preference(config_dir: &Path, binary: &str) {
    let query = [Key::new("binary_preferences"), Key::new(binary)];
    match update_config_with_query::<String>(config_dir, &query, None) {
        Ok(()) => {},
        Err(err) if matches!(err.downcast_ref(), Some(ReadWriteError::NotAUserValue(_))) => {
            debug!(binary, "no saved preference to clear");
        },
        Err(err) => {
            debug!(binary, error = ?err, "failed to clear saved preference");
        },
    }
}

// ---------------------------------------------------------------------------
// Core pipeline
// ---------------------------------------------------------------------------

/// Download a custom catalog package and register a GC root for it.
///
/// Encapsulates the three-step sequence for custom catalog packages:
/// FloxHub auth setup → authenticated `nix copy` → GC root registration.
///
/// # GC root timing
/// The GC root is registered immediately after the `nix copy` completes.
/// There is a brief window between the two calls where a concurrent `nix gc`
/// could evict the just-downloaded paths. In practice `nix gc` must be
/// invoked explicitly and the window is milliseconds, so this is acceptable.
/// A full retry loop (like `materialise_with_retry`) would close it entirely.
async fn download_custom_catalog_package(
    flox: &Flox,
    store_paths: &[String],
    catalog_pkg: &CatalogPackage,
    attr_path: &str,
    pkg_spec: &str,
    gc_root_prefix: &Path,
) -> Result<(), RunError> {
    let auth = NixAuth::from_flox(flox)
        .map_err(|e| RunError::BuildFailed(pkg_spec.to_string(), BuildEnvError::Auth(e)))?;
    let no_netrc_is_error = auth.token().is_none();
    let netrc_guard = auth.try_create_netrc();
    let netrc_path: Option<&Path> = netrc_guard.as_deref();

    let store_locations = flox
        .floxhub_client
        .get_store_info(store_paths.to_vec())
        .await
        .map_err(|e| {
            debug!(error = ?e, "get_store_info failed");
            RunError::CatalogError(pkg_spec.to_string())
        })?;

    {
        let _dl = info_span!(
            "run_download",
            progress = format!("Downloading '{pkg_spec}'...")
        )
        .entered();
        copy_from_custom_catalog_locations(
            store_paths,
            &catalog_pkg.id,
            attr_path,
            &store_locations,
            no_netrc_is_error,
            netrc_path,
        )
        .map_err(|e| RunError::BuildFailed(pkg_spec.to_string(), e))?;
    }

    // TODO: wrap the nix copy + GC root sequence in a materialise_with_retry
    // equivalent to close the race window where nix gc could evict paths
    // between the two calls. The window is milliseconds today, but a retry loop
    // is the correct long-term fix.
    substitute_store_paths(store_paths, Some(gc_root_prefix))
        .map_err(|e| RunError::BuildFailed(pkg_spec.to_string(), e))?;

    Ok(())
}

/// Resolve, download, and exec the requested command.
async fn exec_run(run_args: RunArgs, config: &Config, flox: &Flox) -> Result<()> {
    // 0. Choose the package: explicit `-p`, saved preference, or lookup.
    let selection = select_package(&run_args, config, flox).await?;
    let pkg_spec = selection.pkg_spec().to_string();

    // 1. Parse the package spec and reject unsupported syntax.
    let catalog_pkg = CatalogPackage::from_str(&pkg_spec)
        .map_err(|e| RunError::InvalidPackageSpec(pkg_spec.clone(), e))?;

    validate_plain_package(&catalog_pkg, &pkg_spec)?;

    // Persist explicit choices (validated above) as preferences. A failed
    // write should not stop the run.
    if selection.should_save()
        && let Some(binary) = run_args.executable.to_str()
        && config.flox.binary_preferences.get(binary) != Some(&pkg_spec)
    {
        match save_preference(&flox.config_dir, binary, &pkg_spec) {
            Ok(()) => {
                message::plain(format!(
                    "Saved '{pkg_spec}' as the package for '{binary}'. Use 'flox run --reselect {binary}' to change it."
                ));
            },
            Err(err) => {
                debug!(binary, error = ?err, "failed to save package preference");
            },
        }
    }

    let attr_path = catalog_pkg.pkg_path.clone();
    let version = catalog_pkg.version.clone();

    debug!(
        install_id = %catalog_pkg.id,
        attr_path = %attr_path,
        version = ?version,
        "resolved package spec"
    );

    // 2. Parse the system.
    let system: PackageSystem = flox
        .system
        .parse()
        .map_err(|_| RunError::PackageUnavailableOnSystem(pkg_spec.clone(), flox.system.clone()))?;

    // 3. Build a PackageGroup and call the catalog resolver.
    let descriptor = PackageDescriptor {
        install_id: catalog_pkg.id.clone(),
        attr_path: attr_path.clone(),
        systems: vec![system],
        version,
        allow_broken: None,
        allow_insecure: None,
        allow_missing_builds: None,
        allow_pre_releases: None,
        allow_unfree: None,
        allowed_licenses: None,
        derivation: None,
    };

    let package_group = PackageGroup {
        name: "toplevel".to_string(),
        descriptors: vec![descriptor],
    };

    let mut resolved_groups = flox
        .floxhub_client
        .resolve(vec![package_group])
        .await
        .map_err(|_| RunError::CatalogError(pkg_spec.clone()))?;

    // 4. Extract and classify the resolution result.
    let group = resolved_groups
        .drain(..)
        .next()
        .ok_or_else(|| RunError::CatalogError(pkg_spec.clone()))?;

    // Check for error-level resolution messages before looking at the page.
    for msg in &group.msgs {
        if msg.level() != MessageLevel::Error {
            continue;
        }
        return Err(classify_resolution_message(msg, &pkg_spec, &flox.system).into());
    }

    let page = group
        .page
        .ok_or_else(|| RunError::PackageNotFound(pkg_spec.clone()))?;

    let packages = page.packages.unwrap_or_default();
    if packages.is_empty() {
        return Err(RunError::PackageNotFound(pkg_spec.clone()).into());
    }

    let resolved_pkg = &packages[0];

    debug!(
        pname = %resolved_pkg.pname,
        version = %resolved_pkg.version,
        "package resolved"
    );

    // 5. Collect store paths.
    let outputs_to_install: Vec<String> = resolved_pkg
        .outputs_to_install
        .clone()
        .unwrap_or_else(|| vec!["out".to_string()]);

    let store_paths: Vec<String> = resolved_pkg
        .outputs
        .iter()
        .filter(|o| outputs_to_install.contains(&o.name))
        .map(|o| o.store_path.clone())
        .collect();

    if store_paths.is_empty() {
        return Err(RunError::NoStorePaths(pkg_spec.clone()).into());
    }

    debug!(store_paths = ?store_paths, "store paths to download");

    // 6. Download the package store paths with a stable GC root.
    //
    // The GC root is keyed on system + attr_path so repeated invocations of
    // the same package skip the download. `flox.cache_dir/run` is already
    // reserved as a runtime-dir fallback, hence the `run-gc-roots` name.
    let gc_root_dir = flox.cache_dir.join("run-gc-roots");
    std::fs::create_dir_all(&gc_root_dir)
        .map_err(|e| RunError::CreateGcRootDir(pkg_spec.clone(), e))?;

    let gc_root_prefix = gc_root_dir.join(format!("{}.{}", flox.system, attr_path));

    // Skip if store paths are present AND our GC root symlink already exists.
    // Checking both avoids the case where the store was populated by another
    // process (e.g., an earlier test): we must still register the GC root so
    // `nix store gc` cannot collect the paths out from under us.
    let gc_root_exists = gc_root_prefix.exists();
    let all_present = store_paths.iter().all(|p| Path::new(p).exists());
    if !all_present || !gc_root_exists {
        // TODO: once the async-to-sync boundary is resolved (spawn_blocking or
        // block_on), call realise_lockfile with a 1-element list here instead of
        // download_custom_catalog_package. This would share the semaphore, retry
        // loop, and error handling already proven in the env-build path.
        if catalog_pkg.is_custom_catalog() {
            download_custom_catalog_package(
                flox,
                &store_paths,
                &catalog_pkg,
                &attr_path,
                &pkg_spec,
                &gc_root_prefix,
            )
            .await?;
        } else {
            // Base catalog: try public substituters, fall back to source build.
            //
            // Per-run GC root for source builds; keyed on PID so concurrent
            // runs don't clobber each other's outputs.
            let pid = std::process::id();
            let build_gc_root =
                gc_root_dir.join(format!("build-{}.{}-{}", flox.system, attr_path, pid));

            // Substitution and source-build are both inside the realise closure so
            // materialise_with_retry can retry the whole sequence on a GC race.
            materialise_with_retry(
                || {
                    let ok = {
                        let _dl = info_span!(
                            "run_download",
                            progress = format!("Downloading '{pkg_spec}'...")
                        )
                        .entered();
                        substitute_store_paths(&store_paths, Some(&gc_root_prefix))?
                    };
                    if !ok {
                        // Cache miss; build from source.
                        build_catalog_pkg_from_source(
                            &resolved_pkg.locked_url,
                            &attr_path,
                            &flox.system,
                            resolved_pkg.unfree,
                            resolved_pkg.broken,
                            Some(&build_gc_root),
                        )
                    } else {
                        Ok(())
                    }
                },
                || {
                    // Source-built paths (different hash from catalog) are tracked
                    // via GC root symlinks, not store_paths. If build_gc_root has
                    // symlinks, the source-build path was taken — check those real
                    // output paths. Otherwise, substitution was used — check the
                    // catalog store_paths directly.
                    let gc_paths = collect_store_paths_from_gc_root(&build_gc_root);
                    if gc_paths.is_empty() {
                        store_paths
                            .iter()
                            .filter(|p| std::fs::metadata(p).is_err())
                            .cloned()
                            .collect()
                    } else {
                        gc_paths
                            .into_iter()
                            .filter(|p| std::fs::metadata(p).is_err())
                            .collect()
                    }
                },
                || {
                    let gc_paths = collect_store_paths_from_gc_root(&build_gc_root);
                    if gc_paths.is_empty() {
                        store_paths.clone()
                    } else {
                        gc_paths
                    }
                },
                || Ok::<(), BuildEnvError>(()),
            )
            .map_err(|e| RunError::BuildFailed(pkg_spec.clone(), e))?;

            // Source build was used if the GC root has symlinks; exec via its PATH.
            // Substitution leaves build_gc_root empty — fall through to store_paths exec.
            let build_paths = collect_store_paths_from_gc_root(&build_gc_root);
            if !build_paths.is_empty() {
                // Fork a background watcher that removes the GC root when the
                // exec'd command exits.
                fork_gc_root_watcher(&build_gc_root)
                    .map_err(|e| RunError::ExecFailed("fork gc watcher".into(), e))?;

                let bin_dirs = collect_bin_dirs_from_gc_root(&build_gc_root);
                let new_path = prepend_path_dirs(&bin_dirs);

                debug!(path = ?new_path, "exec via build output PATH");

                let err = std::process::Command::new(&run_args.executable)
                    .args(&run_args.args)
                    .env("PATH", &new_path)
                    .exec();

                return Err(RunError::ExecFailed(
                    run_args.executable.to_string_lossy().into_owned(),
                    err,
                )
                .into());
            }
        }
    }

    // 7. Locate the executable in bin/ then sbin/ of all outputs.
    let executable_path = find_executable(&store_paths, &run_args.executable, &pkg_spec)?;

    debug!(path = %executable_path.display(), "found executable");

    // 8. Exec (replace the flox process).
    let err = std::process::Command::new(&executable_path)
        .args(&run_args.args)
        .exec();

    // exec only returns on error.
    Err(RunError::ExecFailed(executable_path.display().to_string(), err).into())
}

// ---------------------------------------------------------------------------
// Resolution error classification
// ---------------------------------------------------------------------------

/// Map a typed `ResolutionMessage` to the appropriate `RunError`.
fn classify_resolution_message(msg: &ResolutionMessage, pkg_spec: &str, system: &str) -> RunError {
    match msg {
        ResolutionMessage::AttrPathNotFoundNotInCatalog(_) => {
            RunError::PackageNotFound(pkg_spec.to_string())
        },
        ResolutionMessage::AttrPathNotFoundNotFoundForAllSystems(_) => {
            RunError::PackageUnavailableOnSystem(pkg_spec.to_string(), system.to_string())
        },
        other => RunError::ResolutionMessage(pkg_spec.to_string(), other.msg().to_string()),
    }
}

// ---------------------------------------------------------------------------
// Executable discovery
// ---------------------------------------------------------------------------

/// Search `bin/` across all outputs, then `sbin/` across all outputs.
///
/// `bin/` wins overall before `sbin/` is consulted, so the result is
/// deterministic. A candidate must be a regular file with at least one
/// executable bit (`mode & 0o111 != 0`). No fallback to the caller's PATH.
pub fn find_executable(
    store_paths: &[String],
    executable: &OsString,
    pkg_spec: &str,
) -> Result<PathBuf, RunError> {
    // Reject names containing path separators to prevent traversal outside
    // the package's store prefix (e.g. "../../etc/shadow").
    if executable.to_string_lossy().contains('/') {
        return Err(RunError::ExecutableNotFound {
            executable: executable.to_string_lossy().into_owned(),
            package: pkg_spec.to_string(),
        });
    }

    for dir in &["bin", "sbin"] {
        for store_path in store_paths {
            let candidate = Path::new(store_path).join(dir).join(executable);
            if let Ok(meta) = std::fs::metadata(&candidate)
                && meta.is_file()
                && meta.permissions().mode() & 0o111 != 0
            {
                return Ok(candidate);
            }
        }
    }

    Err(RunError::ExecutableNotFound {
        executable: executable.to_string_lossy().into_owned(),
        package: pkg_spec.to_string(),
    })
}

// ---------------------------------------------------------------------------
// Build-from-source helpers
// ---------------------------------------------------------------------------

/// Collect `bin/` directories from build output symlinks rooted at `prefix`.
///
/// `nix build --out-link <prefix>` creates `<prefix>`, `<prefix>-doc`,
/// `<prefix>-dev`, etc. This function scans the parent directory for any
/// entry whose name starts with the file_name component of `prefix`, follows
/// each symlink to its store-path target, and collects any `bin/` subdirs
/// that exist there.
pub fn collect_bin_dirs_from_gc_root(prefix: &Path) -> Vec<PathBuf> {
    let parent = match prefix.parent() {
        Some(p) => p,
        None => return vec![],
    };
    let file_name = match prefix.file_name().and_then(OsStr::to_str) {
        Some(n) => n.to_string(),
        None => return vec![],
    };

    let Ok(entries) = std::fs::read_dir(parent) else {
        return vec![];
    };

    let mut bin_dirs = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !name_str.starts_with(&file_name) {
            continue;
        }
        // Follow the symlink (nix build creates symlinks into the store).
        let target = match std::fs::read_link(entry.path())
            .or_else(|_| std::fs::canonicalize(entry.path()))
        {
            Ok(t) => t,
            Err(_) => continue,
        };
        let bin = target.join("bin");
        if bin.is_dir() {
            bin_dirs.push(bin);
        }
    }
    bin_dirs
}

/// Collect the Nix store-path targets of build output symlinks rooted at `prefix`.
///
/// After `nix build --out-link <prefix>`, symlinks like `<prefix>`,
/// `<prefix>-doc`, `<prefix>-dev` point into the Nix store.  This function
/// returns those store-path strings so callers can check whether they are
/// present on disk (used as the `missing_paths` / `expected_paths` closures
/// passed to `materialise_with_retry`).
pub fn collect_store_paths_from_gc_root(prefix: &Path) -> Vec<String> {
    let parent = match prefix.parent() {
        Some(p) => p,
        None => return vec![],
    };
    let file_name = match prefix.file_name().and_then(OsStr::to_str) {
        Some(n) => n.to_string(),
        None => return vec![],
    };
    let Ok(entries) = std::fs::read_dir(parent) else {
        return vec![];
    };
    entries
        .flatten()
        .filter(|e| {
            let n = e.file_name();
            let s = n.to_string_lossy();
            s == file_name || s.starts_with(&format!("{file_name}-"))
        })
        .filter(|e| e.path().is_symlink())
        .filter_map(|e| std::fs::read_link(e.path()).ok())
        .filter_map(|t| t.to_str().map(|s| s.to_string()))
        .collect()
}

/// Prepend `dirs` to the current `PATH`, returning the combined value.
///
/// Each directory is joined with `:` and the current `PATH` is appended.
pub fn prepend_path_dirs(dirs: &[PathBuf]) -> OsString {
    let current_path = std::env::var_os("PATH").unwrap_or_default();
    let mut parts: Vec<u8> = Vec::new();
    for dir in dirs {
        if !parts.is_empty() {
            parts.push(b':');
        }
        parts.extend_from_slice(dir.as_os_str().as_encoded_bytes());
    }
    if !parts.is_empty() && !current_path.is_empty() {
        parts.push(b':');
    }
    parts.extend_from_slice(current_path.as_encoded_bytes());
    OsString::from_vec(parts)
}

/// Fork a background watcher child that removes `prefix`* symlinks when the
/// parent (exec'd command) exits.
///
/// `exec` preserves the PID, so the command the user invoked keeps this
/// process's PID. The watcher polls `getppid()`: while it still reports that
/// PID the parent is alive, and once the parent exits the watcher is reparented
/// (to init or a subreaper) and `getppid()` changes. The watcher then removes
/// all symlinks whose name starts with `prefix.file_name()` in the same
/// directory, and exits.
///
/// Polling `getppid()` is a cheap syscall and, unlike a recorded PID compared
/// with `kill(pid, 0)`, cannot be fooled by PID reuse: the reparent is what is
/// observed, not the liveness of an arbitrary PID.
///
/// This ensures temporary GC-root symlinks created by `nix build --out-link`
/// are cleaned up even though we `exec` into the target command and can no
/// longer run cleanup code ourselves.
pub fn fork_gc_root_watcher(gc_root_prefix: &Path) -> Result<(), std::io::Error> {
    use std::thread::sleep;
    use std::time::Duration;

    use nix::unistd::{ForkResult, fork, getppid};

    // The exec'd command keeps this process's PID, so capture it before the
    // fork as the parent the watcher should wait on.
    let expected_parent = std::process::id() as i32;

    match unsafe { fork() }.map_err(std::io::Error::from)? {
        ForkResult::Child => {
            // Poll until the parent (exec'd command) exits. `getppid()` stops
            // reporting `expected_parent` once the parent dies and the watcher
            // is reparented. If the parent already exited (e.g. exec failed),
            // the condition is false on the first check and cleanup runs
            // immediately.
            while getppid().as_raw() == expected_parent {
                sleep(Duration::from_millis(500));
            }

            // Parent exited. Remove GC root symlinks.
            if let (Some(parent), Some(file_name)) =
                (gc_root_prefix.parent(), gc_root_prefix.file_name())
            {
                let scan_prefix = file_name.to_string_lossy().into_owned();
                if let Ok(entries) = std::fs::read_dir(parent) {
                    for entry in entries.flatten() {
                        let name = entry.file_name();
                        let name_str = name.to_string_lossy();
                        if name_str.starts_with(&scan_prefix) && entry.path().is_symlink() {
                            let _ = std::fs::remove_file(entry.path());
                        }
                    }
                }
            }

            // Use _exit, not exit: after fork() the child must not run
            // atexit handlers or flush stdio buffers shared with the parent.
            unsafe { nix::libc::_exit(0) };
        },
        ForkResult::Parent { .. } => Ok(()),
    }
}

// ---------------------------------------------------------------------------
// Help
// ---------------------------------------------------------------------------

/// Print a hand-written synopsis for `flox run`.
///
/// bpaf's built-in help is suppressed because the catchall struct consumes
/// `--help` before bpaf can render it. This function matches bpaf's stdout
/// convention so callers cannot tell the difference.
pub fn print_help() {
    print!(indoc! {"
        Run a command from a Flox Catalog package

        Usage: flox run [-p <PACKAGE>] [--reselect] <COMMAND> [--] [ARGS...]

        Options:
          -p, --package <PACKAGE>   Package that provides the command.
                                    Accepts a version constraint (e.g. curl@8.0).
              --reselect            Clear the saved package preference for the
                                    command and choose again.
          -h, --help                Print this help

        Package selection:
          Without '--package', the command name is looked up in the Flox Catalog
          to find the package that provides it. If several packages provide the
          command, a package named exactly like the command wins; otherwise an
          interactive prompt asks you to choose. Choices made with '--package'
          or the prompt are saved as preferences and reused silently.

        Use '--' between the command name and its arguments when the
        arguments contain flags, and before the command name if the name
        itself starts with '-'.

        Examples:
          flox run hello
          flox run readelf -- -a /bin/ls
          flox run --reselect vi
          flox run -p curl@8.0 curl -- -sL http://example.com
          flox run hello -- --version

        Limitations:
          Output selectors (^) are not supported.

        Caching:
          Downloaded store paths are registered as GC roots under
          $FLOX_CACHE_DIR/run-gc-roots/. Repeated invocations of the same package
          skip the download step.

        Run 'man flox-run' for more details.
    "});
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    #[cfg(unix)]
    use std::os::unix::ffi::OsStringExt;

    use super::*;

    fn os(s: &str) -> OsString {
        OsString::from(s)
    }

    fn os_vec(v: &[&str]) -> Vec<OsString> {
        v.iter().map(OsString::from).collect()
    }

    // -----------------------------------------------------------------------
    // parse_run_args tests
    // -----------------------------------------------------------------------

    #[test]
    fn package_flag_short() {
        let result =
            parse_run_args(os_vec(&["-p", "binutils", "readelf", "-a", "/bin/ls"])).unwrap();
        assert_eq!(
            result,
            ParsedArgs::Run(RunArgs {
                package: Some("binutils".to_string()),
                reselect: false,
                executable: os("readelf"),
                args: os_vec(&["-a", "/bin/ls"]),
            })
        );
    }

    #[test]
    fn package_flag_long() {
        let result = parse_run_args(os_vec(&["--package", "binutils", "readelf"])).unwrap();
        assert_eq!(
            result,
            ParsedArgs::Run(RunArgs {
                package: Some("binutils".to_string()),
                reselect: false,
                executable: os("readelf"),
                args: vec![],
            })
        );
    }

    #[test]
    fn double_dash_before_executable() {
        let result = parse_run_args(os_vec(&["-p", "somepkg", "--", "-weirdname"])).unwrap();
        assert_eq!(
            result,
            ParsedArgs::Run(RunArgs {
                package: Some("somepkg".to_string()),
                reselect: false,
                executable: os("-weirdname"),
                args: vec![],
            })
        );
    }

    #[test]
    fn double_dash_after_command_is_dropped_as_separator() {
        // A single `--` right after the command is the args separator.
        let result = parse_run_args(os_vec(&["-p", "x", "cmd", "--", "-z"])).unwrap();
        assert_eq!(
            result,
            ParsedArgs::Run(RunArgs {
                package: Some("x".to_string()),
                reselect: false,
                executable: os("cmd"),
                args: os_vec(&["-z"]),
            })
        );
    }

    #[test]
    fn second_double_dash_after_command_stays_in_passthrough() {
        // Only the first `--` after the command is a separator; a literal
        // `--` can still be passed by writing it after the separator.
        let result = parse_run_args(os_vec(&["cmd", "--", "--", "-z"])).unwrap();
        assert_eq!(
            result,
            ParsedArgs::Run(RunArgs {
                package: None,
                reselect: false,
                executable: os("cmd"),
                args: os_vec(&["--", "-z"]),
            })
        );
    }

    #[test]
    fn separator_dropped_after_forced_positional_command() {
        // The separator rule also applies when the command name itself was
        // introduced with a leading `--`.
        let result = parse_run_args(os_vec(&["--", "-weirdname", "--", "-z"])).unwrap();
        assert_eq!(
            result,
            ParsedArgs::Run(RunArgs {
                package: None,
                reselect: false,
                executable: os("-weirdname"),
                args: os_vec(&["-z"]),
            })
        );
    }

    #[test]
    fn version_after_separator_stays_in_passthrough() {
        // Canonical form for passing `--version` to the command.
        let result = parse_run_args(os_vec(&["hello", "--", "--version"])).unwrap();
        assert_eq!(
            result,
            ParsedArgs::Run(RunArgs {
                package: None,
                reselect: false,
                executable: os("hello"),
                args: os_vec(&["--version"]),
            })
        );
    }

    #[test]
    fn no_args_returns_no_executable_error() {
        let result = parse_run_args(vec![]);
        assert!(matches!(result, Err(RunError::NoExecutable)));
    }

    #[test]
    fn bare_command_parses_without_package() {
        // Without -p/--package the command name is kept for the lookup flow.
        let result = parse_run_args(os_vec(&["curl", "http://example.com"])).unwrap();
        assert_eq!(
            result,
            ParsedArgs::Run(RunArgs {
                package: None,
                reselect: false,
                executable: os("curl"),
                args: os_vec(&["http://example.com"]),
            })
        );
    }

    #[test]
    fn posix_order_dependence_p_after_command_stays_in_passthrough() {
        // After the first positional `curl`, -p belongs to curl (not flox).
        let result = parse_run_args(os_vec(&["curl", "-p", "curl"])).unwrap();
        assert_eq!(
            result,
            ParsedArgs::Run(RunArgs {
                package: None,
                reselect: false,
                executable: os("curl"),
                args: os_vec(&["-p", "curl"]),
            })
        );
    }

    #[test]
    fn reselect_flag_before_command() {
        let result = parse_run_args(os_vec(&["--reselect", "vi"])).unwrap();
        assert_eq!(
            result,
            ParsedArgs::Run(RunArgs {
                package: None,
                reselect: true,
                executable: os("vi"),
                args: vec![],
            })
        );
    }

    #[test]
    fn reselect_after_command_stays_in_passthrough() {
        let result = parse_run_args(os_vec(&["vi", "--reselect"])).unwrap();
        assert_eq!(
            result,
            ParsedArgs::Run(RunArgs {
                package: None,
                reselect: false,
                executable: os("vi"),
                args: os_vec(&["--reselect"]),
            })
        );
    }

    #[test]
    fn unknown_flag_returns_error() {
        let result = parse_run_args(os_vec(&["--unknown", "curl"]));
        assert!(matches!(result, Err(RunError::UnknownFlag(_))));
    }

    #[test]
    fn equals_form_long_rejected() {
        let result = parse_run_args(os_vec(&["--package=binutils", "readelf"]));
        assert!(matches!(result, Err(RunError::UnknownFlag(_))));
    }

    #[test]
    fn equals_form_short_rejected() {
        let result = parse_run_args(os_vec(&["-p=binutils", "readelf"]));
        assert!(matches!(result, Err(RunError::UnknownFlag(_))));
    }

    #[test]
    fn bundled_short_form_rejected() {
        let result = parse_run_args(os_vec(&["-pbinutils", "readelf"]));
        assert!(matches!(result, Err(RunError::UnknownFlag(_))));
    }

    #[test]
    fn help_short_before_positional() {
        let result = parse_run_args(os_vec(&["-h"])).unwrap();
        assert_eq!(result, ParsedArgs::Help);
    }

    #[test]
    fn help_long_before_positional() {
        let result = parse_run_args(os_vec(&["--help"])).unwrap();
        assert_eq!(result, ParsedArgs::Help);
    }

    #[test]
    fn help_after_package_before_command_is_intercepted() {
        // `flox run -p curl --help` — help is before the command.
        let result = parse_run_args(os_vec(&["-p", "curl", "--help"])).unwrap();
        assert_eq!(result, ParsedArgs::Help);
    }

    #[test]
    fn help_after_command_stays_in_passthrough() {
        // `flox run -p curl curl --help` — help is after the command name (curl).
        let result = parse_run_args(os_vec(&["-p", "curl", "curl", "--help"])).unwrap();
        assert_eq!(
            result,
            ParsedArgs::Run(RunArgs {
                package: Some("curl".to_string()),
                reselect: false,
                executable: os("curl"),
                args: os_vec(&["--help"]),
            })
        );
    }

    #[test]
    fn help_after_double_dash_stays_in_passthrough() {
        // `--` forces positional mode, so `--help` after it goes to command.
        let result = parse_run_args(os_vec(&["-p", "hello", "--", "hello", "--help"])).unwrap();
        assert_eq!(
            result,
            ParsedArgs::Run(RunArgs {
                package: Some("hello".to_string()),
                reselect: false,
                executable: os("hello"),
                args: os_vec(&["--help"]),
            })
        );
    }

    #[test]
    fn missing_package_value_short() {
        let result = parse_run_args(os_vec(&["-p"]));
        assert!(matches!(result, Err(RunError::MissingPackageValue(_))));
    }

    #[test]
    fn missing_package_value_long() {
        let result = parse_run_args(os_vec(&["--package"]));
        assert!(matches!(result, Err(RunError::MissingPackageValue(_))));
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_package_value() {
        let bad = OsString::from_vec(vec![0xff]);
        let args = vec![os("-p"), bad, os("cmd")];
        let result = parse_run_args(args);
        assert!(matches!(result, Err(RunError::PackageSpecNotUtf8)));
    }

    // -----------------------------------------------------------------------
    // validate_plain_package tests
    // -----------------------------------------------------------------------

    #[test]
    fn validate_plain_package_accepts_simple() {
        let pkg: CatalogPackage = "cowsay".parse().unwrap();
        assert!(validate_plain_package(&pkg, "cowsay").is_ok());
    }

    #[test]
    fn validate_plain_package_accepts_dotted() {
        let pkg: CatalogPackage = "python3Packages.requests".parse().unwrap();
        assert!(validate_plain_package(&pkg, "python3Packages.requests").is_ok());
    }

    #[test]
    fn validate_plain_package_accepts_version() {
        let pkg: CatalogPackage = "curl@8.0".parse().unwrap();
        assert!(validate_plain_package(&pkg, "curl@8.0").is_ok());
    }

    #[test]
    fn validate_plain_package_rejects_outputs() {
        let pkg: CatalogPackage = "foo^bin".parse().unwrap();
        assert!(matches!(
            validate_plain_package(&pkg, "foo^bin"),
            Err(RunError::UnsupportedPackageSpec(_))
        ));
    }

    #[test]
    fn validate_plain_package_accepts_custom_catalog() {
        let pkg: CatalogPackage = "mycatalog/vim".parse().unwrap();
        assert!(validate_plain_package(&pkg, "mycatalog/vim").is_ok());
    }

    // -----------------------------------------------------------------------
    // Binary-to-package selection tests
    // -----------------------------------------------------------------------

    fn candidate(attr_path: &str, pname: &str) -> PackageCandidate {
        PackageCandidate {
            attr_path: attr_path.to_string(),
            pname: pname.to_string(),
            description: None,
            version: None,
        }
    }

    #[test]
    fn dedupe_candidates_keeps_first_occurrence() {
        let candidates = vec![
            PackageCandidate {
                version: Some("9.1".to_string()),
                ..candidate("vim", "vim")
            },
            PackageCandidate {
                version: Some("8.2".to_string()),
                ..candidate("vim", "vim")
            },
            candidate("neovim", "neovim"),
        ];
        assert_eq!(dedupe_candidates(candidates), vec![
            PackageCandidate {
                version: Some("9.1".to_string()),
                ..candidate("vim", "vim")
            },
            candidate("neovim", "neovim"),
        ]);
    }

    #[test]
    fn candidate_display_shows_differing_pname() {
        let display = candidate("binutils", "binutils-wrapper").to_string();
        assert_eq!(display, "binutils (binutils-wrapper)");
    }

    #[test]
    fn candidate_display_shows_version_and_truncated_description() {
        let display = PackageCandidate {
            attr_path: "vim".to_string(),
            pname: "vim".to_string(),
            description: Some("x".repeat(70)),
            version: Some("9.1".to_string()),
        }
        .to_string();
        assert_eq!(display, format!("vim (9.1) — {}...", "x".repeat(57)));
    }

    #[test]
    fn ambiguous_binary_error_lists_packages_inline() {
        let err = RunError::AmbiguousBinary {
            binary: "vi".to_string(),
            package_list: "vim, neovim, vimer".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "Multiple packages provide 'vi' and no preference is saved.\n\
             Packages with this binary: vim, neovim, vimer\n\
             Use 'flox run --package <PACKAGE> vi' to specify a package."
        );
    }

    #[test]
    fn binary_not_found_error_suggests_package_flag() {
        let err = RunError::BinaryNotFound("frobnicate".to_string());
        assert_eq!(
            err.to_string(),
            "No packages found that provide 'frobnicate'.\n\
             Use 'flox run --package <PACKAGE> frobnicate' to specify the package directly."
        );
    }

    #[test]
    fn save_and_clear_preference_roundtrip() {
        let tmp = tempfile::TempDir::new().unwrap();

        save_preference(tmp.path(), "vi", "vim").unwrap();
        let config_contents =
            std::fs::read_to_string(tmp.path().join(flox_config::FLOX_CONFIG_FILE)).unwrap();
        assert!(config_contents.contains("[binary_preferences]"));
        assert!(config_contents.contains("vi = \"vim\""));

        clear_preference(tmp.path(), "vi");
        let config_contents =
            std::fs::read_to_string(tmp.path().join(flox_config::FLOX_CONFIG_FILE)).unwrap();
        assert!(!config_contents.contains("vi = \"vim\""));

        // Clearing a preference that does not exist is not an error.
        clear_preference(tmp.path(), "never-saved");
    }

    #[test]
    fn save_preference_keeps_dotted_binary_name_as_single_key() {
        let tmp = tempfile::TempDir::new().unwrap();

        save_preference(tmp.path(), "python3.12", "python312").unwrap();
        let config_contents =
            std::fs::read_to_string(tmp.path().join(flox_config::FLOX_CONFIG_FILE)).unwrap();
        assert!(config_contents.contains("\"python3.12\" = \"python312\""));
    }

    // -----------------------------------------------------------------------
    // find_executable tests
    // -----------------------------------------------------------------------

    #[test]
    fn find_executable_in_bin_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        let bin_dir = tmp.path().join("bin");
        std::fs::create_dir(&bin_dir).unwrap();
        let exe_path = bin_dir.join("hello");
        std::fs::write(&exe_path, "#!/bin/sh\necho hello").unwrap();
        // Set executable bit.
        let mut perms = std::fs::metadata(&exe_path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&exe_path, perms).unwrap();

        let store_path = tmp.path().to_string_lossy().to_string();
        let result = find_executable(&[store_path], &os("hello"), "hello").unwrap();
        assert_eq!(result, exe_path);
    }

    #[test]
    fn find_executable_skips_non_executable_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let bin_dir = tmp.path().join("bin");
        std::fs::create_dir(&bin_dir).unwrap();
        let path = bin_dir.join("hello");
        std::fs::write(&path, "#!/bin/sh").unwrap();
        // No executable bit.
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o644);
        std::fs::set_permissions(&path, perms).unwrap();

        let store_path = tmp.path().to_string_lossy().to_string();
        let result = find_executable(&[store_path], &os("hello"), "hello");
        assert!(matches!(result, Err(RunError::ExecutableNotFound { .. })));
    }

    #[test]
    fn find_executable_sbin_fallback() {
        let tmp = tempfile::TempDir::new().unwrap();
        let sbin_dir = tmp.path().join("sbin");
        std::fs::create_dir(&sbin_dir).unwrap();
        let exe_path = sbin_dir.join("arp");
        std::fs::write(&exe_path, "#!/bin/sh").unwrap();
        let mut perms = std::fs::metadata(&exe_path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&exe_path, perms).unwrap();

        let store_path = tmp.path().to_string_lossy().to_string();
        let result = find_executable(&[store_path], &os("arp"), "net-tools").unwrap();
        assert_eq!(result, exe_path);
    }

    #[test]
    fn find_executable_bin_wins_over_sbin() {
        let tmp = tempfile::TempDir::new().unwrap();
        let bin_dir = tmp.path().join("bin");
        let sbin_dir = tmp.path().join("sbin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        std::fs::create_dir_all(&sbin_dir).unwrap();

        let bin_path = bin_dir.join("tool");
        let sbin_path = sbin_dir.join("tool");
        for p in &[&bin_path, &sbin_path] {
            std::fs::write(p, "#!/bin/sh").unwrap();
            let mut perms = std::fs::metadata(p).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(p, perms).unwrap();
        }

        let store_path = tmp.path().to_string_lossy().to_string();
        let result = find_executable(&[store_path], &os("tool"), "somepkg").unwrap();
        assert_eq!(result, bin_path);
    }

    #[test]
    fn find_executable_not_found() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store_path = tmp.path().to_string_lossy().to_string();
        let result = find_executable(&[store_path], &os("missing"), "mypkg");
        assert!(matches!(result, Err(RunError::ExecutableNotFound { .. })));
    }

    #[test]
    fn find_executable_second_output() {
        let tmp1 = tempfile::TempDir::new().unwrap();
        let tmp2 = tempfile::TempDir::new().unwrap();
        let sp1 = tmp1.path().to_string_lossy().to_string();
        let sp2 = tmp2.path().to_string_lossy().to_string();

        let bin_dir2 = tmp2.path().join("bin");
        std::fs::create_dir(&bin_dir2).unwrap();
        let exe_path = bin_dir2.join("readelf");
        std::fs::write(&exe_path, "#!/bin/sh").unwrap();
        let mut perms = std::fs::metadata(&exe_path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&exe_path, perms).unwrap();

        let result = find_executable(&[sp1, sp2], &os("readelf"), "binutils").unwrap();
        assert_eq!(result, exe_path);
    }

    // -----------------------------------------------------------------------
    // collect_bin_dirs_from_gc_root tests
    // -----------------------------------------------------------------------

    #[test]
    fn collect_bin_dirs_finds_bin_under_symlink() {
        let tmp = tempfile::TempDir::new().unwrap();

        // Simulate a nix store output: a real directory with a bin/ subdir.
        let store_out = tmp.path().join("store-out");
        let bin_dir = store_out.join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();

        // Create a symlink that looks like nix build --out-link output.
        let prefix = tmp.path().join("build-aarch64-darwin.hello-42");
        std::os::unix::fs::symlink(&store_out, &prefix).unwrap();

        let result = collect_bin_dirs_from_gc_root(&prefix);
        assert_eq!(result, vec![bin_dir]);
    }

    #[test]
    fn collect_bin_dirs_collects_suffix_symlinks() {
        let tmp = tempfile::TempDir::new().unwrap();

        // Simulate nix build creating multiple output symlinks with the same
        // prefix: <prefix>, <prefix>-doc, <prefix>-dev.
        let prefix = tmp.path().join("build-aarch64-darwin.pkg-99");

        for suffix in &["", "-doc", "-dev"] {
            let store_out = tmp.path().join(format!("store-out{suffix}"));
            let bin = store_out.join("bin");
            std::fs::create_dir_all(&bin).unwrap();
            let link = tmp
                .path()
                .join(format!("build-aarch64-darwin.pkg-99{suffix}"));
            std::os::unix::fs::symlink(&store_out, &link).unwrap();
        }

        let mut result = collect_bin_dirs_from_gc_root(&prefix);
        result.sort();
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn collect_bin_dirs_skips_outputs_without_bin() {
        let tmp = tempfile::TempDir::new().unwrap();

        let store_out = tmp.path().join("store-out-no-bin");
        // No bin/ subdir.
        std::fs::create_dir_all(&store_out).unwrap();

        let prefix = tmp.path().join("build-aarch64-darwin.no-bin-42");
        std::os::unix::fs::symlink(&store_out, &prefix).unwrap();

        let result = collect_bin_dirs_from_gc_root(&prefix);
        assert!(result.is_empty());
    }

    // -----------------------------------------------------------------------
    // prepend_path_dirs tests
    // -----------------------------------------------------------------------

    #[test]
    fn prepend_path_dirs_prepends_to_existing_path() {
        // We cannot rely on the process-level PATH in tests; check structure.
        let dirs = vec![PathBuf::from("/my/bin"), PathBuf::from("/other/bin")];
        let result = prepend_path_dirs(&dirs);
        let result_str = result.to_string_lossy();
        assert!(result_str.starts_with("/my/bin:/other/bin"));
    }

    #[test]
    fn prepend_path_dirs_empty_dirs_returns_existing_path() {
        let result = prepend_path_dirs(&[]);
        // When no dirs are passed, the result should equal the current PATH.
        let current = std::env::var_os("PATH").unwrap_or_default();
        assert_eq!(result, current);
    }
}
