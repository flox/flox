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
use flox_config::Config;
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
    ByCommandError,
    CatalogClientTrait,
    CommandProvider,
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

use crate::commands::general::{remove_config_key_with_query, update_config_with_query};
use crate::subcommand_metric;
use crate::utils::message;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum number of candidates shown when the disambiguation prompt lists
/// packages. Beyond this count a "(N shown, M total)" line is appended.
const DISAMBIGUATION_LIMIT: usize = 10;

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

/// Errors specific to `flox run`.
#[derive(Debug, Error)]
pub enum RunError {
    /// No command was given after parsing all flags.
    #[error(
        "No command specified.\n\
         Run 'flox run --package <PACKAGE> <COMMAND> [ARGS...]'."
    )]
    NoExecutable,

    /// `-p`/`--package` was absent (reported before `NoExecutable`).
    #[error(
        "No package specified.\n\
         Use '--package <PACKAGE>' to specify the package that provides the command."
    )]
    MissingPackage,

    /// `-p`/`--package` flag appeared without a value.
    #[error(
        "Missing value for '{0}'.\n\
         Use '--package <PACKAGE>' to specify the package that provides the command."
    )]
    MissingPackageValue(String),

    /// The value passed to `-p`/`--package` was not valid UTF-8.
    #[error("Package specs must be valid UTF-8.")]
    PackageSpecNotUtf8,

    /// `--reselect` appeared without a value.
    #[error(
        "Missing value for '--reselect'.\n\
         Use '--reselect <COMMAND>' to clear the saved package preference for a command."
    )]
    MissingReselectValue,

    /// The value passed to `--reselect` was not valid UTF-8.
    #[error("Command names must be valid UTF-8.")]
    CommandNameNotUtf8,

    /// `--reselect` was combined with a command to run.
    #[error(
        "'--reselect' cannot be combined with a command to run.\n\
         Run 'flox run --reselect <COMMAND>' on its own to clear a saved package preference."
    )]
    ReselectWithCommand,

    /// `CatalogPackage::from_str` failed.
    #[error(
        "Invalid package '{0}'.\n\
         Use 'flox search' to find available packages."
    )]
    InvalidPackageSpec(String, #[source] RawManifestError),

    /// Package spec uses unsupported syntax (`@`, `^`).
    #[error(
        "Unsupported package '{0}'.\n\
         'flox run' accepts a plain package name or custom catalog package; \
         version constraints ('@') and output selectors ('^') are not supported."
    )]
    UnsupportedPackageSpec(String),

    /// An unrecognised flag appeared before the command name.
    #[error(
        "Unknown option '{0}'.\n\
         Use '--' before the command name if it starts with '-'."
    )]
    UnknownFlag(String),

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

    /// The catalog lookup timed out or returned a transport error.
    #[error(
        "Could not reach the Flox Catalog to look up '{command}'.\n\
         Use 'flox run --package <PACKAGE> {command}' to run it directly."
    )]
    LookupUnavailable { command: String },

    /// The catalog is indexed but no package provides this command.
    #[error(
        "No package provides the command '{command}'.\n\
         Use 'flox run --package <PACKAGE> {command}' if you know the package."
    )]
    NoCommandProvider { command: String },

    /// The command is not in the current command index (listing not yet scraped).
    #[error(
        "The command '{command}' has not been indexed yet.\n\
         Use 'flox run --package <PACKAGE> {command}' to run it directly."
    )]
    CommandNotIndexed { command: String },

    /// The command name fails catalog validation (e.g. too short or too long)
    /// so it cannot be looked up. This is a client-side check — the catalog
    /// was never contacted.
    #[error(
        "'{command}' is not a valid catalog command name.\n\
         Use 'flox run --package <PACKAGE> {command}' to run it directly."
    )]
    InvalidCommandName { command: String },

    /// Multiple packages provide the command and the session is non-interactive.
    #[error("{}", render_ambiguous(command, providers, *total))]
    AmbiguousCommandNonInteractive {
        command: String,
        providers: Vec<CommandProvider>,
        /// Catalog-reported total count of providers, used to render the
        /// "(N shown, M total)" trailer when the list is truncated.
        total: u64,
    },

    /// Writing the user's package preference to the config file failed.
    ///
    /// Not yet used — T7 (DEV-185) will call this when persisting a
    /// disambiguation selection at the interactive prompt.
    #[allow(dead_code)]
    #[error("Failed to save the package preference for '{0}'.")]
    PreferenceWriteFailed(String, #[source] anyhow::Error),
}

// ---------------------------------------------------------------------------
// Parsed argument types
// ---------------------------------------------------------------------------

/// Outcome of the `parse_run_args` state machine.
#[derive(Debug, Clone, PartialEq)]
pub enum ParsedArgs {
    /// `-h`/`--help` was seen before the first positional or `--`.
    Help,
    /// `--reselect <COMMAND>` was seen; clear that command's saved preference
    /// and run nothing.
    Reselect(String),
    /// A fully-specified run invocation.
    Run(RunArgs),
}

/// Validated arguments produced by the POSIX state machine.
#[derive(Debug, Clone, PartialEq)]
pub struct RunArgs {
    /// Package spec from `-p`/`--package` (plain form only).
    ///
    /// `None` when no `-p` was given. Parsing accepts that; [`exec_run`]
    /// rejects it, because it is the only consumer that needs a package.
    pub package: Option<String>,
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
            ParsedArgs::Reselect(command) => {
                subcommand_metric!("run::reselect");
                let cleared = clear_run_preference(&config.flox.config_dir, &command)?;
                if cleared {
                    message::updated(format!(
                        "Cleared the saved package preference for '{command}'."
                    ));
                } else {
                    message::updated(format!("No saved package preference for '{command}'."));
                }
                Ok(())
            },
            ParsedArgs::Run(run_args) => {
                let pkg_spec = resolve_command(&run_args, &config, &flox)
                    .await
                    .map_err(anyhow::Error::from)?;
                let resolved = RunArgs {
                    package: Some(pkg_spec),
                    ..run_args
                };
                exec_run(resolved, &flox).await
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Ambiguous-command rendering
// ---------------------------------------------------------------------------

/// Format the list of candidates for `AmbiguousCommandNonInteractive`.
///
/// Renders up to `DISAMBIGUATION_LIMIT` entries and appends a count line when
/// `total` exceeds the limit. `total` is passed separately so the caller can
/// supply either the actual catalog total or `providers.len()`.
fn render_ambiguous(command: &str, providers: &[CommandProvider], total: u64) -> String {
    use std::fmt::Write as _;
    let mut s = format!("Multiple packages provide '{command}'.\n");
    for p in providers.iter().take(DISAMBIGUATION_LIMIT) {
        let _ = writeln!(s, "  {:<12} ({})", p.pname, p.attr_path);
    }
    if total > DISAMBIGUATION_LIMIT as u64 {
        let _ = writeln!(s, "  ... ({DISAMBIGUATION_LIMIT} shown, {total} total)");
    }
    let _ = write!(
        s,
        "Use 'flox run --package <PACKAGE> {command}' to choose one."
    );
    s
}

// ---------------------------------------------------------------------------
// run_preferences config helpers
// ---------------------------------------------------------------------------

/// Build the `run_preferences.<command>` key path.
///
/// Both segments are `Key::new`, never `Key::parse`: a command name can
/// contain `.`, and dotted-key parsing would shatter it into nested tables
/// instead of writing one literal key. `flox-config`'s
/// `writing_auto_activate_preference_for_path_with_dot` in `src/write.rs` is
/// the regression test for the equivalent hazard on the auto-activation path.
fn run_preference_query(command: &str) -> [Key; 2] {
    [Key::new("run_preferences"), Key::new(command)]
}

/// Record the package the user chose to provide `command`.
///
/// Only the interactive disambiguation prompt calls this; see
/// [`flox_config::FloxConfig::run_preferences`]. That prompt does not exist
/// yet, so nothing but the tests calls this today — the writer ships with the
/// config field so the two stay in step.
#[allow(dead_code)]
pub fn write_run_preference(config_dir: &Path, command: &str, attr_path: &str) -> Result<()> {
    update_config_with_query(config_dir, &run_preference_query(command), Some(attr_path))?;
    Ok(())
}

/// Forget the package the user chose to provide `command`.
///
/// Returns whether an entry was actually removed. Clearing an absent
/// preference is a success, not an error: the caller asked to be prompted
/// again next time, and that is already true.
pub fn clear_run_preference(config_dir: &Path, command: &str) -> Result<bool> {
    remove_config_key_with_query(config_dir, &run_preference_query(command))
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
/// - `--` → force positional mode; next arg is the command even if it starts with `-`
/// - `--reselect` (space form only) → consume next arg as a command name
/// - any other `"-…"` → `UnknownFlag`
///
/// After the first positional (or after `--`), everything is forwarded
/// verbatim including any literal `--`.
///
/// A missing `-p` is not an error here — [`RunArgs::package`] is optional and
/// [`exec_run`] enforces it. Only a missing command is rejected.
pub fn parse_run_args(args: Vec<OsString>) -> Result<ParsedArgs, RunError> {
    let mut package: Option<String> = None;
    let mut reselect: Option<String> = None;
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
                passthrough.extend(iter);
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
                let value_os = iter.next().ok_or(RunError::MissingReselectValue)?;
                let value = value_os
                    .into_string()
                    .map_err(|_| RunError::CommandNameNotUtf8)?;
                reselect = Some(value);
            },
            Some(s) if s.starts_with('-') => {
                return Err(RunError::UnknownFlag(s.to_owned()));
            },
            _ => {
                // First non-flag positional is the command name; everything
                // after it is passthrough verbatim (including any literal `--`).
                executable = Some(arg);
                passthrough.extend(iter);
                break 'flags;
            },
        }
    }

    // `--reselect` is a one-shot config edit, so it cannot also name a command
    // to run.
    if let Some(command) = reselect {
        if executable.is_some() {
            return Err(RunError::ReselectWithCommand);
        }
        return Ok(ParsedArgs::Reselect(command));
    }

    let executable = executable.ok_or(RunError::NoExecutable)?;

    Ok(ParsedArgs::Run(RunArgs {
        package,
        executable,
        args: passthrough,
    }))
}

// ---------------------------------------------------------------------------
// Package spec validation
// ---------------------------------------------------------------------------

/// Reject package specs that use unsupported syntax.
///
/// Accepts a plain attr-path (`cowsay`, `python3Packages.requests`) or a
/// custom catalog package (`mycatalog/vim`). Version constraints (`@`) and
/// output selectors (`^`) are not supported.
pub fn validate_plain_package(pkg: &CatalogPackage, raw: &str) -> Result<(), RunError> {
    if pkg.version.is_some() || pkg.outputs.is_some() {
        return Err(RunError::UnsupportedPackageSpec(raw.to_string()));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Command resolver
// ---------------------------------------------------------------------------

/// Resolve a bare command name to a package attr-path, or pass a `-p` spec
/// through unchanged.
///
/// Implements a 7-branch funnel:
///
/// 1. `-p` supplied → return it directly, no catalog call.
/// 2. Saved preference in config → return silently.
/// 3. `by_command` lookup with a 5-second timeout.
///    - `listing_known=false`, empty providers → `CommandNotIndexed`.
///    - `listing_known=true`, empty providers → `NoCommandProvider`.
/// 4. Single provider → return its `attr_path` silently.
/// 5. Exactly one `exact_name_match=true` → return its `attr_path` silently.
/// 6. Multiple candidates → `AmbiguousCommandNonInteractive` for now.
///    T7 will split into a TTY prompt and a non-interactive fail path.
async fn resolve_command(
    run_args: &RunArgs,
    config: &Config,
    flox: &Flox,
) -> Result<String, RunError> {
    // Branch 1: -p supplied — bypass resolver entirely.
    if let Some(pkg) = &run_args.package {
        return Ok(pkg.clone());
    }

    let command = run_args.executable.to_string_lossy().into_owned();

    // Branch 2: saved preference — return silently, no write.
    if let Some(attr_path) = config.flox.run_preferences.get(&command) {
        debug!(branch = "saved", %command, "resolve_command");
        return Ok(attr_path.clone());
    }

    // Branch 3: call by_command; networking timeouts are handled by the client.
    // system.try_into() only fails for unrecognized system strings, which
    // cannot occur in practice — Flox validates the system at startup.
    let system: PackageSystem = flox
        .system
        .clone()
        .try_into()
        .expect("flox.system is always a valid PackageSystem");

    let result = flox
        .floxhub_client
        .by_command(&command, system)
        .await
        .map_err(|e| classify_by_command_error(e, command.clone()))?;

    // Branches 3a/3b: empty provider list — tristate on listing_known.
    if result.providers.is_empty() {
        return Err(if result.listing_known {
            RunError::NoCommandProvider { command }
        } else {
            RunError::CommandNotIndexed { command }
        });
    }

    // Branch 4: single provider — silent, no write.
    if result.providers.len() == 1 {
        debug!(branch = "single", %command, "resolve_command");
        return Ok(result.providers[0].attr_path.clone());
    }

    // Branch 5: exactly one exact_name_match — silent, no write.
    let exact: Vec<_> = result
        .providers
        .iter()
        .filter(|p| p.exact_name_match)
        .collect();
    if exact.len() == 1 {
        debug!(branch = "exact", %command, "resolve_command");
        return Ok(exact[0].attr_path.clone());
    }

    // Branches 6/7: multiple candidates.
    // T7 will split this into a TTY prompt (branch 6) and the non-interactive
    // error path (branch 7). For now all multi-candidate cases return the
    // non-interactive error so the stub compiles and tests can exercise it.
    debug!(
        branch = "ambiguous",
        %command,
        count = result.providers.len(),
        "resolve_command"
    );
    Err(RunError::AmbiguousCommandNonInteractive {
        command,
        providers: result.providers,
        total: result.total_count as u64,
    })
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
    let no_netrc_is_error = !auth.has_credential();
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
async fn exec_run(run_args: RunArgs, flox: &Flox) -> Result<()> {
    let pkg_spec = run_args.package.clone().ok_or(RunError::MissingPackage)?;

    // 1. Parse the package spec and reject unsupported syntax.
    let catalog_pkg = CatalogPackage::from_str(&pkg_spec)
        .map_err(|e| RunError::InvalidPackageSpec(pkg_spec.clone(), e))?;

    validate_plain_package(&catalog_pkg, &pkg_spec)?;

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

/// Map a `ByCommandError` to the most accurate `RunError`.
///
/// `InvalidCommandName` is a client-side validation error that fires before
/// any network request — the catalog's `Name` type requires 2–200 characters,
/// so very short command names (e.g. `w`) cannot be queried at all.
///
/// `FloxhubClientError` covers transport failures and server errors.
fn classify_by_command_error(err: ByCommandError, command: String) -> RunError {
    match err {
        ByCommandError::InvalidCommandName(_) => RunError::InvalidCommandName { command },
        ByCommandError::FloxhubClientError(e) => {
            debug!(error = ?e, %command, "by_command lookup failed");
            RunError::LookupUnavailable { command }
        },
    }
}

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

        Usage: flox run -p <PACKAGE> -- <COMMAND> [ARGS...]
               flox run --reselect <COMMAND>

        Options:
          -p, --package <PACKAGE>   Package that provides the command (required)
              --reselect <COMMAND>  Forget the saved package preference for a command
          -h, --help                Print this help

        Always use '--' to separate flox flags from the command and its arguments.
        This matches 'flox activate -- <command>' and ensures flags like '--version'
        reach the command rather than flox.

        Examples:
          flox run -p curl -- curl http://example.com
          flox run -p binutils -- readelf -a /bin/ls
          flox run -p hello -- hello --help
          flox run -p hello -- hello --version

        Limitations:
          Version constraints (@) and output selectors (^) are not supported.
          The -p flag is always required to run a command.

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

    use flox_config::FLOX_CONFIG_FILE;

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
                executable: os("-weirdname"),
                args: vec![],
            })
        );
    }

    #[test]
    fn double_dash_after_command_stays_in_passthrough() {
        // A literal `--` after the command stays in passthrough.
        let result = parse_run_args(os_vec(&["-p", "x", "cmd", "--", "-z"])).unwrap();
        assert_eq!(
            result,
            ParsedArgs::Run(RunArgs {
                package: Some("x".to_string()),
                executable: os("cmd"),
                args: os_vec(&["--", "-z"]),
            })
        );
    }

    #[test]
    fn no_args_returns_no_executable_error() {
        // Parsing no longer rejects a missing -p, so the command name is the
        // only thing left to complain about.
        let result = parse_run_args(vec![]);
        assert!(matches!(result, Err(RunError::NoExecutable)));
    }

    #[test]
    fn no_package_flag_parses_with_none_package() {
        // A bare command with no -p/--package parses; exec_run rejects it.
        let result = parse_run_args(os_vec(&["curl", "http://example.com"])).unwrap();
        assert_eq!(
            result,
            ParsedArgs::Run(RunArgs {
                package: None,
                executable: os("curl"),
                args: os_vec(&["http://example.com"]),
            })
        );
    }

    #[test]
    fn posix_order_dependence_curl_minus_p_curl() {
        // After the first positional `curl`, -p belongs to curl (not flox),
        // so flox is left with no package of its own.
        let result = parse_run_args(os_vec(&["curl", "-p", "curl"])).unwrap();
        assert_eq!(
            result,
            ParsedArgs::Run(RunArgs {
                package: None,
                executable: os("curl"),
                args: os_vec(&["-p", "curl"]),
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

    #[test]
    fn reselect_parses_to_reselect() {
        let result = parse_run_args(os_vec(&["--reselect", "vi"])).unwrap();
        assert_eq!(result, ParsedArgs::Reselect("vi".to_string()));
    }

    #[test]
    fn reselect_without_value_rejected() {
        let result = parse_run_args(os_vec(&["--reselect"]));
        assert!(matches!(result, Err(RunError::MissingReselectValue)));
    }

    #[test]
    fn reselect_with_command_rejected() {
        let result = parse_run_args(os_vec(&["--reselect", "vi", "vim"]));
        assert!(matches!(result, Err(RunError::ReselectWithCommand)));
    }

    #[test]
    fn reselect_equals_form_rejected() {
        let result = parse_run_args(os_vec(&["--reselect=vi"]));
        assert!(matches!(result, Err(RunError::UnknownFlag(_))));
    }

    #[test]
    fn reselect_after_command_stays_in_passthrough() {
        // `flox run vi --reselect foo` — after the first positional,
        // `--reselect` belongs to the command, not to flox.
        let result = parse_run_args(os_vec(&["-p", "vim", "vi", "--reselect", "foo"])).unwrap();
        assert_eq!(
            result,
            ParsedArgs::Run(RunArgs {
                package: Some("vim".to_string()),
                executable: os("vi"),
                args: os_vec(&["--reselect", "foo"]),
            })
        );
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_reselect_value() {
        let bad = OsString::from_vec(vec![0xff]);
        let args = vec![os("--reselect"), bad];
        let result = parse_run_args(args);
        assert!(matches!(result, Err(RunError::CommandNameNotUtf8)));
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
    // run_preferences config helper tests
    // -----------------------------------------------------------------------

    fn config_contents(config_dir: &Path) -> String {
        std::fs::read_to_string(config_dir.join(FLOX_CONFIG_FILE)).unwrap()
    }

    /// Read back the `[run_preferences]` table.
    ///
    /// `FloxConfig` cannot be deserialized from the user config file alone —
    /// it carries resolved fields like `cache_dir` that only the full config
    /// assembly supplies — so assert against the TOML directly.
    fn run_preferences_table(config_dir: &Path) -> toml::Table {
        let document: toml::Table = toml::from_str(&config_contents(config_dir)).unwrap();
        document
            .get("run_preferences")
            .and_then(|value| value.as_table())
            .cloned()
            .unwrap_or_default()
    }

    #[test]
    fn write_run_preference_writes_command_as_one_literal_key() {
        let dir = tempfile::tempdir().unwrap();
        // A dotted command name must land as a single quoted key rather than
        // being shattered into `[run_preferences.my]` / `tool`.
        write_run_preference(dir.path(), "my.tool", "python311Packages.pip").unwrap();

        let contents = config_contents(dir.path());
        assert!(
            contents.contains(r#""my.tool" = "python311Packages.pip""#),
            "expected one literal key, got:\n{contents}"
        );
    }

    #[test]
    fn write_run_preference_round_trips_dotted_attr_path() {
        let dir = tempfile::tempdir().unwrap();
        write_run_preference(dir.path(), "pip", "python311Packages.pip").unwrap();

        let expected: toml::Table = toml::from_str(r#"pip = "python311Packages.pip""#).unwrap();
        assert_eq!(run_preferences_table(dir.path()), expected);
    }

    #[test]
    fn clear_run_preference_removes_existing_key() {
        let dir = tempfile::tempdir().unwrap();
        write_run_preference(dir.path(), "vi", "vim").unwrap();

        assert!(clear_run_preference(dir.path(), "vi").unwrap());

        // Clearing the last entry leaves an empty `[run_preferences]` table
        // behind. `auto_activate_environments` behaves the same way; the
        // contents are what matters.
        assert_eq!(run_preferences_table(dir.path()), toml::Table::new());
    }

    #[test]
    fn clear_run_preference_absent_key_in_existing_table_is_ok_false() {
        let dir = tempfile::tempdir().unwrap();
        // `[run_preferences]` exists but holds a different command, so the
        // parent-segment walk in `write_to` succeeds and only the leaf is
        // missing.
        write_run_preference(dir.path(), "vi", "vim").unwrap();

        assert!(!clear_run_preference(dir.path(), "emacs").unwrap());
    }

    #[test]
    fn clear_run_preference_absent_table_is_ok_false() {
        let dir = tempfile::tempdir().unwrap();
        // No config file at all: `write_to` creates the `[run_preferences]`
        // table on the way down, then fails to find the leaf. Nothing reaches
        // disk on that path.
        assert!(!clear_run_preference(dir.path(), "vi").unwrap());
        assert!(!dir.path().join(FLOX_CONFIG_FILE).exists());
    }

    #[test]
    fn remove_config_key_with_query_propagates_read_failures() {
        let dir = tempfile::tempdir().unwrap();
        // A directory where the config file belongs makes the read fail with
        // something other than NotFound, which must not be flattened into
        // `Ok(false)`.
        std::fs::create_dir(dir.path().join(FLOX_CONFIG_FILE)).unwrap();

        let err = clear_run_preference(dir.path(), "vi").unwrap_err();
        assert!(
            err.to_string()
                .contains("Could not read current config file"),
            "unexpected error: {err}"
        );
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
    fn validate_plain_package_rejects_version() {
        let pkg: CatalogPackage = "curl@8.0".parse().unwrap();
        assert!(matches!(
            validate_plain_package(&pkg, "curl@8.0"),
            Err(RunError::UnsupportedPackageSpec(_))
        ));
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

    // -----------------------------------------------------------------------
    // resolve_command tests
    // -----------------------------------------------------------------------

    use flox_rust_sdk::flox::test_helpers::flox_instance;
    use flox_rust_sdk::providers::catalog::test_helpers::{
        UNIT_TEST_GENERATED,
        catalog_replay_client,
    };

    /// Build a minimal Config with an optional saved run preference.
    fn make_config(
        run_preferences: std::collections::HashMap<String, String>,
    ) -> flox_config::Config {
        flox_config::Config {
            flox: flox_config::FloxConfig {
                run_preferences,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    // Branch 1: -p supplied — the resolver never contacts the catalog.
    #[tokio::test(flavor = "multi_thread")]
    async fn return_package_spec_when_dash_p_supplied() {
        // No catalog call — client is never exercised here.
        let (flox, _dir) = flox_instance();
        let config = make_config(Default::default());
        let run_args = RunArgs {
            package: Some("curl".to_string()),
            executable: OsString::from("curl"),
            args: vec![],
        };

        let result = resolve_command(&run_args, &config, &flox).await.unwrap();
        assert_eq!(result, "curl");
    }

    // Branch 2: saved preference — the resolver returns it without a network call.
    #[tokio::test(flavor = "multi_thread")]
    async fn return_saved_preference_when_present() {
        let (flox, _dir) = flox_instance();
        let mut prefs = std::collections::HashMap::new();
        prefs.insert("vi".to_string(), "vim".to_string());
        let config = make_config(prefs);
        let run_args = RunArgs {
            package: None,
            executable: OsString::from("vi"),
            args: vec![],
        };

        let result = resolve_command(&run_args, &config, &flox).await.unwrap();
        assert_eq!(result, "vim");
    }

    // Branch 3 error: by_command returns an HTTP 500 → LookupUnavailable.
    #[tokio::test(flavor = "multi_thread")]
    async fn return_lookup_unavailable_on_by_command_error() {
        let (mut flox, _dir) = flox_instance();
        flox.floxhub_client =
            catalog_replay_client(UNIT_TEST_GENERATED.join("resolve_command_lookup_error.yaml"))
                .await;
        let config = make_config(Default::default());
        let run_args = RunArgs {
            package: None,
            executable: OsString::from("curl"),
            args: vec![],
        };

        let err = resolve_command(&run_args, &config, &flox)
            .await
            .unwrap_err();
        assert!(
            matches!(err, RunError::LookupUnavailable { .. }),
            "expected LookupUnavailable, got: {err:?}"
        );
    }

    // Branch 3a: listing_known=false, providers=[] → CommandNotIndexed.
    #[tokio::test(flavor = "multi_thread")]
    async fn return_command_not_indexed_when_listing_unknown() {
        let (mut flox, _dir) = flox_instance();
        flox.floxhub_client =
            catalog_replay_client(UNIT_TEST_GENERATED.join("resolve_command_not_indexed.yaml"))
                .await;
        let config = make_config(Default::default());
        let run_args = RunArgs {
            package: None,
            executable: OsString::from("curl"),
            args: vec![],
        };

        let err = resolve_command(&run_args, &config, &flox)
            .await
            .unwrap_err();
        assert!(
            matches!(err, RunError::CommandNotIndexed { .. }),
            "expected CommandNotIndexed, got: {err:?}"
        );
    }

    // Branch 3b: listing_known=true, providers=[] → NoCommandProvider.
    #[tokio::test(flavor = "multi_thread")]
    async fn return_no_command_provider_when_listing_known_empty() {
        let (mut flox, _dir) = flox_instance();
        flox.floxhub_client = catalog_replay_client(
            UNIT_TEST_GENERATED.join("resolve_command_no_provider_known.yaml"),
        )
        .await;
        let config = make_config(Default::default());
        let run_args = RunArgs {
            package: None,
            executable: OsString::from("curl"),
            args: vec![],
        };

        let err = resolve_command(&run_args, &config, &flox)
            .await
            .unwrap_err();
        assert!(
            matches!(err, RunError::NoCommandProvider { .. }),
            "expected NoCommandProvider, got: {err:?}"
        );
    }

    // Branch 4: single provider → return its attr_path silently.
    #[tokio::test(flavor = "multi_thread")]
    async fn return_attr_path_for_single_provider() {
        let (mut flox, _dir) = flox_instance();
        flox.floxhub_client =
            catalog_replay_client(UNIT_TEST_GENERATED.join("resolve_command_single_provider.yaml"))
                .await;
        let config = make_config(Default::default());
        let run_args = RunArgs {
            package: None,
            executable: OsString::from("curl"),
            args: vec![],
        };

        let result = resolve_command(&run_args, &config, &flox).await.unwrap();
        assert_eq!(result, "curlFull");
    }

    // Branch 5: two providers, one exact_name_match=true → return exact match.
    #[tokio::test(flavor = "multi_thread")]
    async fn return_exact_match_when_one_exact_name_match() {
        let (mut flox, _dir) = flox_instance();
        flox.floxhub_client =
            catalog_replay_client(UNIT_TEST_GENERATED.join("resolve_command_exact_match.yaml"))
                .await;
        let config = make_config(Default::default());
        let run_args = RunArgs {
            package: None,
            executable: OsString::from("curl"),
            args: vec![],
        };

        let result = resolve_command(&run_args, &config, &flox).await.unwrap();
        assert_eq!(result, "curlFull");
    }

    // Multiple exact_name_match=true → ambiguous (branch 6/7 stub).
    #[tokio::test(flavor = "multi_thread")]
    async fn return_ambiguous_when_two_exact_name_matches() {
        let (mut flox, _dir) = flox_instance();
        flox.floxhub_client = catalog_replay_client(
            UNIT_TEST_GENERATED.join("resolve_command_ambiguous_two_exact.yaml"),
        )
        .await;
        let config = make_config(Default::default());
        let run_args = RunArgs {
            package: None,
            executable: OsString::from("curl"),
            args: vec![],
        };

        let err = resolve_command(&run_args, &config, &flox)
            .await
            .unwrap_err();
        assert!(
            matches!(err, RunError::AmbiguousCommandNonInteractive { .. }),
            "expected AmbiguousCommandNonInteractive, got: {err:?}"
        );
    }

    // Multiple providers, none exact → ambiguous.
    #[tokio::test(flavor = "multi_thread")]
    async fn return_ambiguous_when_multiple_no_exact() {
        let (mut flox, _dir) = flox_instance();
        flox.floxhub_client = catalog_replay_client(
            UNIT_TEST_GENERATED.join("resolve_command_ambiguous_no_exact.yaml"),
        )
        .await;
        let config = make_config(Default::default());
        let run_args = RunArgs {
            package: None,
            executable: OsString::from("curl"),
            args: vec![],
        };

        let err = resolve_command(&run_args, &config, &flox)
            .await
            .unwrap_err();
        assert!(
            matches!(err, RunError::AmbiguousCommandNonInteractive { .. }),
            "expected AmbiguousCommandNonInteractive, got: {err:?}"
        );
    }

    // The NoCommandProvider error message must not suggest 'flox search' because
    // command-index lookup and package search are independent catalogs.
    #[tokio::test(flavor = "multi_thread")]
    async fn no_command_provider_message_does_not_mention_flox_search() {
        let (mut flox, _dir) = flox_instance();
        flox.floxhub_client = catalog_replay_client(
            UNIT_TEST_GENERATED.join("resolve_command_no_provider_known.yaml"),
        )
        .await;
        let config = make_config(Default::default());
        let run_args = RunArgs {
            package: None,
            executable: OsString::from("curl"),
            args: vec![],
        };

        let err = resolve_command(&run_args, &config, &flox)
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(
            !msg.contains("flox search"),
            "NoCommandProvider message must not suggest 'flox search': {msg}"
        );
    }

    // A single-character command like "w" fails the catalog's Name validation
    // (minimum 2 chars) before any network request. resolve_command should
    // surface InvalidCommandName, not LookupUnavailable or CommandNotIndexed.
    #[tokio::test(flavor = "multi_thread")]
    async fn single_char_command_gives_invalid_name_error() {
        let (flox, _dir) = flox_instance();
        let config = make_config(Default::default());
        let run_args = RunArgs {
            package: None,
            executable: OsString::from("w"),
            args: vec![],
        };

        let err = resolve_command(&run_args, &config, &flox)
            .await
            .unwrap_err();
        assert!(
            matches!(err, RunError::InvalidCommandName { .. }),
            "expected InvalidCommandName for single-char command, got: {err:?}"
        );
    }
}
