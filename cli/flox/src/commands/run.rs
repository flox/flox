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

use std::ffi::OsString;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::Result;
use bpaf::Bpaf;
use flox_manifest::raw::{CatalogPackage, RawManifestError};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::providers::buildenv::{BuildEnvError, substitute_store_paths};
use floxhub_client::{
    CatalogClientTrait,
    MessageLevel,
    PackageDescriptor,
    PackageGroup,
    PackageSystem,
    ResolutionMessage,
};
use thiserror::Error;
use tracing::{debug, info_span};

use crate::subcommand_metric;

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

    /// `CatalogPackage::from_str` failed.
    #[error(
        "Invalid package '{0}'.\n\
         Use 'flox search' to find available packages."
    )]
    InvalidPackageSpec(String, #[source] RawManifestError),

    /// Package spec uses syntax not supported in phase 1 (`@`, `^`, `/`).
    #[error(
        "Unsupported package '{0}'.\n\
         'flox run' accepts a plain package name; version constraints ('@'), \
         output selectors ('^'), and custom catalogs ('/') are not supported."
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

    /// The `nix build` invocation to download store paths failed at the process level.
    #[error("Failed to run nix while downloading package '{0}'.")]
    Substitute(String, #[source] BuildEnvError),

    /// Substitution returned `false` (nix build reported failure).
    #[error(
        "Failed to download package '{0}'.\n\
         Check your network connection and try again."
    )]
    DownloadFailed(String),

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
    /// Package spec from `-p`/`--package` (required, plain form only).
    pub package: String,
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
    pub async fn handle(self, flox: Flox) -> Result<()> {
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
            ParsedArgs::Run(run_args) => exec_run(run_args, &flox).await,
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
/// - `--` → force positional mode; next arg is the command even if it starts with `-`
/// - any other `"-…"` → `UnknownFlag`
///
/// After the first positional (or after `--`), everything is forwarded
/// verbatim including any literal `--`.
///
/// After the loop: missing `-p` is reported before missing command.
pub fn parse_run_args(args: Vec<OsString>) -> Result<ParsedArgs, RunError> {
    let mut package: Option<String> = None;
    let mut executable: Option<OsString> = None;
    let mut passthrough: Vec<OsString> = Vec::new();

    let mut iter = args.into_iter();

    // Phase 1: scan flags until we see `--` or the first positional.
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

    // Report missing package before missing command.
    let package = package.ok_or(RunError::MissingPackage)?;
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

/// Reject package specs that use syntax unsupported in phase 1.
///
/// Accepts only a plain attr-path (e.g. `cowsay`, `python3Packages.requests`).
/// Version constraints (`@`), output selectors (`^`), and custom catalogs
/// (`/`) are not supported and cause `UnsupportedPackageSpec`.
///
/// Custom catalog rejection also makes the substituter-only download path
/// below sufficient: private catalogs require the buildenv realise path
/// (`nix copy --from` + catalog auth) rather than the substituter path.
pub fn validate_plain_package(pkg: &CatalogPackage, raw: &str) -> Result<(), RunError> {
    if pkg.version.is_some() || pkg.outputs.is_some() || pkg.is_custom_catalog() {
        return Err(RunError::UnsupportedPackageSpec(raw.to_string()));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Core pipeline
// ---------------------------------------------------------------------------

/// Resolve, download, and exec the requested command.
async fn exec_run(run_args: RunArgs, flox: &Flox) -> Result<()> {
    let pkg_spec = run_args.package.clone();

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
        name: "run".to_string(),
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

    // 6. Download via the SDK's substitution path with a stable GC root.
    //
    // The GC root is keyed on system + attr_path so repeated invocations of
    // the same package skip the download. `flox.cache_dir/run` is already
    // reserved as a runtime-dir fallback, hence the `run-gc-roots` name.
    let gc_root_dir = flox.cache_dir.join("run-gc-roots");
    std::fs::create_dir_all(&gc_root_dir)
        .map_err(|e| RunError::CreateGcRootDir(pkg_spec.clone(), e))?;

    let gc_root_prefix = gc_root_dir.join(format!("{}.{}", flox.system, attr_path));

    // Skip download if all store paths are already present on disk. Nix store
    // paths are content-addressed, so existence implies correctness. The GC
    // root registered on the first run keeps them from being collected.
    let all_present = store_paths.iter().all(|p| Path::new(p).exists());
    if !all_present {
        let _span = info_span!(
            "run_download",
            progress = format!("Downloading '{pkg_spec}'...")
        )
        .entered();
        let ok = substitute_store_paths(&store_paths, Some(&gc_root_prefix))
            .map_err(|e| RunError::Substitute(pkg_spec.clone(), e))?;
        drop(_span);
        if !ok {
            return Err(RunError::DownloadFailed(pkg_spec.clone()).into());
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
    use std::os::unix::fs::PermissionsExt;

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
// Help
// ---------------------------------------------------------------------------

/// Print a hand-written synopsis for `flox run`.
///
/// bpaf's built-in help is suppressed because the catchall struct consumes
/// `--help` before bpaf can render it. This function matches bpaf's stdout
/// convention so callers cannot tell the difference.
pub fn print_help() {
    println!(
        "Run a command from a Flox Catalog package

Usage: flox run -p <PACKAGE> [--] <COMMAND> [ARGS...]

Options:
  -p, --package <PACKAGE>   Package to run the command from (required)
  -h, --help                Print this help

Flags before the command name belong to flox run; flags after the command name
belong to the command. Use '--' to separate them explicitly, which is required
for commands whose first argument starts with '-'.

Examples:
  flox run -p curl curl http://example.com
  flox run -p binutils readelf -a /bin/ls
  flox run -p hello -- hello --help
  flox run -p hello -- hello --version

Note: '--version' is intercepted by flox unless '--' is used first.

Limitations (phase 1):
  Version constraints (@), output selectors (^), and custom catalogs (/) are
  not supported. The -p flag is always required.

Caching:
  Downloaded store paths are registered as GC roots under
  $FLOX_CACHE_DIR/run-gc-roots/. Repeated invocations of the same package
  skip the download step.

Run 'man flox-run' for more details."
    );
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

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
                package: "binutils".to_string(),
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
                package: "binutils".to_string(),
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
                package: "somepkg".to_string(),
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
                package: "x".to_string(),
                executable: os("cmd"),
                args: os_vec(&["--", "-z"]),
            })
        );
    }

    #[test]
    fn no_args_returns_missing_package_error() {
        let result = parse_run_args(vec![]);
        assert!(matches!(result, Err(RunError::MissingPackage)));
    }

    #[test]
    fn no_package_flag_returns_missing_package() {
        // A bare command with no -p/--package must report MissingPackage.
        let result = parse_run_args(os_vec(&["curl", "http://example.com"]));
        assert!(matches!(result, Err(RunError::MissingPackage)));
    }

    #[test]
    fn posix_order_dependence_curl_minus_p_curl() {
        // After the first positional `curl`, -p belongs to curl (not flox).
        // The absence of a flox -p flag should yield MissingPackage.
        let result = parse_run_args(os_vec(&["curl", "-p", "curl"]));
        assert!(matches!(result, Err(RunError::MissingPackage)));
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
                package: "curl".to_string(),
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
                package: "hello".to_string(),
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
        use std::os::unix::ffi::OsStringExt;
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
    fn validate_plain_package_rejects_custom_catalog() {
        let pkg: CatalogPackage = "mycatalog/vim".parse().unwrap();
        assert!(matches!(
            validate_plain_package(&pkg, "mycatalog/vim"),
            Err(RunError::UnsupportedPackageSpec(_))
        ));
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
    // Replay-client resolution tests. These verify the ADR-011 classification
    // path (typed ResolutionMessage matching) without a live Nix store.
    // Gated behind extra-tests because catalog_replay_client requires the
    // full httpmock infrastructure that the feature enables.
    // -----------------------------------------------------------------------

    #[cfg(feature = "extra-tests")]
    #[tokio::test(flavor = "multi_thread")]
    async fn resolve_package_finds_hello() {
        use flox_rust_sdk::flox::test_helpers::flox_instance;
        use flox_rust_sdk::providers::catalog::test_helpers::catalog_replay_client;
        use flox_test_utils::GENERATED_DATA;

        let (mut flox, _tempdir) = flox_instance();
        flox.floxhub_client =
            catalog_replay_client(GENERATED_DATA.join("resolve/hello.yaml")).await;

        let run_args = RunArgs {
            package: "hello".to_string(),
            executable: os("hello"),
            args: vec![],
        };

        let catalog_pkg: CatalogPackage = run_args.package.parse().unwrap();
        let attr_path = catalog_pkg.pkg_path.clone();
        let system: PackageSystem = flox.system.parse().unwrap();
        let descriptor = PackageDescriptor {
            install_id: catalog_pkg.id.clone(),
            attr_path: attr_path.clone(),
            systems: vec![system],
            version: None,
            allow_broken: None,
            allow_insecure: None,
            allow_missing_builds: None,
            allow_pre_releases: None,
            allow_unfree: None,
            allowed_licenses: None,
            derivation: None,
        };
        let package_group = PackageGroup {
            name: "run".to_string(),
            descriptors: vec![descriptor],
        };

        let resolved = flox
            .floxhub_client
            .resolve(vec![package_group])
            .await
            .unwrap();

        let group = &resolved[0];
        let page = group.page.as_ref().unwrap();
        let pkg = &page.packages.as_ref().unwrap()[0];
        assert_eq!(pkg.pname, "hello");
    }

    #[cfg(feature = "extra-tests")]
    #[tokio::test(flavor = "multi_thread")]
    async fn resolve_package_classifies_not_found() {
        use flox_rust_sdk::flox::test_helpers::flox_instance;
        use flox_rust_sdk::providers::catalog::test_helpers::catalog_replay_client;
        use flox_test_utils::GENERATED_DATA;

        let (mut flox, _tempdir) = flox_instance();
        flox.floxhub_client =
            catalog_replay_client(GENERATED_DATA.join("resolve/failed_resolution.yaml")).await;

        let run_args = RunArgs {
            package: "nonexistent-xyz-package".to_string(),
            executable: os("nonexistent-xyz-package"),
            args: vec![],
        };

        let result = exec_run(run_args, &flox).await;
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("was not found") || msg.contains("not found"),
            "expected not-found error, got: {msg}"
        );
    }
}
