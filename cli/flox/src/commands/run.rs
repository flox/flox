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
use std::io::{BufRead as _, IsTerminal as _};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::Result;
use bpaf::Bpaf;
use flox_manifest::raw::{CatalogPackage, RawManifestError};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::providers::buildenv::{
    BuildEnvError,
    build_catalog_pkg_from_source,
    substitute_store_paths,
};
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

    /// User declined the offer to build the package from source.
    #[error("Cancelled building '{0}' from source.")]
    BuildDeclined(String),

    /// The `nix build` invocation for building from source failed.
    #[error("Failed to build '{0}' from source.\n{1}")]
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

    // 6. Download via the SDK's substitution path with a stable GC root.
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
        let _span = info_span!(
            "run_download",
            progress = format!("Downloading '{pkg_spec}'...")
        )
        .entered();
        let ok = substitute_store_paths(&store_paths, Some(&gc_root_prefix))
            .map_err(|e| RunError::Substitute(pkg_spec.clone(), e))?;
        drop(_span);
        if !ok {
            // Package is not in the binary cache. Offer to build from source.
            //
            // Interactive sessions prompt the user; non-interactive sessions
            // (scripts, CI) proceed automatically to avoid blocking.
            if std::io::stderr().is_terminal() {
                eprint!(
                    "⚠️  Package '{pkg_spec}' is not in the binary cache and must be built \
                     from source. This may take a while. Continue? [Y/n] "
                );
                let stdin = std::io::stdin();
                let mut input = String::new();
                stdin.lock().read_line(&mut input)?;
                let trimmed = input.trim().to_lowercase();
                if trimmed == "n" || trimmed == "no" {
                    return Err(RunError::BuildDeclined(pkg_spec.clone()).into());
                }
            }

            // Use a per-run GC root keyed on system + attr_path + PID so
            // concurrent runs don't clobber each other's build outputs.
            let pid = std::process::id();
            let build_gc_root =
                gc_root_dir.join(format!("build-{}.{}-{}", flox.system, attr_path, pid));

            {
                let _span = info_span!(
                    "run_build",
                    progress = format!("Building '{pkg_spec}' from source...")
                )
                .entered();
                build_catalog_pkg_from_source(
                    &resolved_pkg.locked_url,
                    &attr_path,
                    &flox.system,
                    resolved_pkg.unfree,
                    resolved_pkg.broken,
                    Some(&build_gc_root),
                )
                .map_err(|e| RunError::BuildFailed(pkg_spec.clone(), e))?;
                drop(_span);
            }

            // Fork a background watcher that removes the temporary GC root
            // symlinks when the exec'd command exits.
            fork_gc_root_watcher(&build_gc_root)
                .map_err(|e| RunError::ExecFailed("fork gc watcher".into(), e))?;

            // Locate the executable from the build output symlinks.
            let bin_dirs = collect_bin_dirs_from_gc_root(&build_gc_root);
            let new_path = prepend_path_dirs(&bin_dirs);

            debug!(path = ?new_path, "exec via build output PATH");

            let err = std::process::Command::new(&run_args.executable)
                .args(&run_args.args)
                .env("PATH", &new_path)
                .exec();

            return Err(
                RunError::ExecFailed(run_args.executable.to_string_lossy().into_owned(), err)
                    .into(),
            );
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
        let target = match std::fs::read_link(entry.path()).or_else(|_| {
            std::fs::canonicalize(entry.path())
        }) {
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

/// Prepend `dirs` to the current `PATH`, returning the combined value.
///
/// Each directory is joined with `:` and the current `PATH` is appended.
pub fn prepend_path_dirs(dirs: &[PathBuf]) -> OsString {
    use std::os::unix::ffi::OsStringExt as _;

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
/// The parent holds the write end of a pipe open across the `exec` call.
/// When the exec'd command exits, the write end is closed, causing the child's
/// blocking read to return EOF. On EOF the child removes all symlinks whose
/// name starts with `prefix.file_name()` in the same directory, then exits.
///
/// This ensures temporary GC-root symlinks created by `nix build --out-link`
/// are cleaned up even though we `exec` into the target command and can no
/// longer run cleanup code ourselves.
pub fn fork_gc_root_watcher(gc_root_prefix: &Path) -> Result<(), std::io::Error> {
    use std::os::unix::io::IntoRawFd as _;

    use nix::unistd::{ForkResult, fork};

    // Obtain raw file descriptors before fork so both parent and child can
    // operate on them independently without Rust ownership conflicts.
    let (read_owned, write_owned) = nix::unistd::pipe()?;

    // Clear FD_CLOEXEC on the write end so it survives the exec() call and
    // stays open until the exec'd command exits.
    let flags =
        nix::fcntl::fcntl(&write_owned, nix::fcntl::FcntlArg::F_GETFD).map_err(|e| {
            std::io::Error::other(format!("fcntl F_GETFD: {e}"))
        })?;
    let new_flags =
        nix::fcntl::FdFlag::from_bits_retain(flags) & !nix::fcntl::FdFlag::FD_CLOEXEC;
    nix::fcntl::fcntl(&write_owned, nix::fcntl::FcntlArg::F_SETFD(new_flags)).map_err(|e| {
        std::io::Error::other(format!("fcntl F_SETFD: {e}"))
    })?;

    // Convert to raw fds before fork; each process is responsible for closing
    // its own copies. After fork, Rust's drop semantics do not apply — both
    // processes hold identical fd numbers that are independent OS handles.
    let read_fd = read_owned.into_raw_fd();
    let write_fd = write_owned.into_raw_fd();

    match unsafe { fork() }.map_err(std::io::Error::from)? {
        ForkResult::Child => {
            // Child: close write end, then block until parent (exec'd command) closes it.
            // SAFETY: raw fd is valid in this process; we own it after fork.
            unsafe { nix::libc::close(write_fd) };
            let mut buf = [0u8; 1];
            // Blocking read — returns 0 (EOF) when the last write-end holder exits.
            unsafe { nix::libc::read(read_fd, buf.as_mut_ptr().cast(), 1) };
            unsafe { nix::libc::close(read_fd) };

            // EOF means the exec'd process exited. Remove GC root symlinks.
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

            std::process::exit(0);
        },
        ForkResult::Parent { .. } => {
            // Parent: close read end. Write end stays open — it will be inherited
            // by the exec'd command and closed when that process exits.
            // SAFETY: raw fd is valid in this process.
            unsafe { nix::libc::close(read_fd) };
            // write_fd intentionally left open for exec() inheritance.
            Ok(())
        },
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

        Options:
          -p, --package <PACKAGE>   Package that provides the command (required)
          -h, --help                Print this help

        Always use '--' to separate flox flags from the command and its arguments.
        This matches 'flox activate -- <command>' and ensures flags like '--version'
        reach the command rather than flox.

        Examples:
          flox run -p curl -- curl http://example.com
          flox run -p binutils -- readelf -a /bin/ls
          flox run -p hello -- hello --help
          flox run -p hello -- hello --version

        Limitations (phase 1):
          Version constraints (@), output selectors (^), and custom catalogs (/) are
          not supported. The -p flag is always required.

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
            let link = tmp.path().join(format!("build-aarch64-darwin.pkg-99{suffix}"));
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
