//! External-subcommand resolution and dispatch.
//!
//! [`try_dispatch_external`] is the whole `flox <name>` fallback: resolve
//! `<name>` to a `flox-<name>` executable and replace the current process
//! with it. The extension inherits the caller's environment untouched.
//!
//! Resolution ([`find`]) searches the managed extensions directory first,
//! then `$PATH`. The managed-dir-first order is intentional: the managed
//! dir is populated only by `flox extension install`, so precedence there
//! reflects explicit user intent rather than ambient shell state.

use std::ffi::{OsStr, OsString};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use thiserror::Error;
use tracing::{debug, warn};

use super::manifest::{InstalledState, parse_installed_state};

#[derive(Debug, Error)]
pub enum FindError {
    #[error("no extension named '{0}' is installed")]
    NotFound(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Resolve `flox-<name>` by searching the managed extensions directory and
/// then `$PATH`.
///
/// `extensions_root` is typically `flox.data_dir.join("extensions")`. The
/// managed layout is `extensions_root/<flox-name>/<flox-name>` (one
/// subdirectory per installed extension). `path_env` is the raw value of
/// `$PATH`; pass `None` to skip the PATH fallback.
pub fn find(
    name: &str,
    extensions_root: &Path,
    path_env: Option<&OsStr>,
) -> Result<PathBuf, FindError> {
    if name.is_empty() || name.contains('/') || name.contains(std::path::MAIN_SEPARATOR) {
        return Err(FindError::NotFound(name.to_owned()));
    }
    let exe_name = format!("flox-{name}");

    let managed = extensions_root.join(&exe_name).join(&exe_name);
    if is_executable(&managed)? {
        return Ok(managed);
    }

    if let Some(path) = path_env {
        for dir in std::env::split_paths(path) {
            let candidate = dir.join(&exe_name);
            if is_executable(&candidate)? {
                return Ok(candidate);
            }
        }
    }

    Err(FindError::NotFound(name.to_owned()))
}

#[cfg(unix)]
fn is_executable(p: &Path) -> std::io::Result<bool> {
    use std::os::unix::fs::PermissionsExt;
    match std::fs::metadata(p) {
        Ok(md) if md.is_file() && md.permissions().mode() & 0o111 != 0 => Ok(true),
        Ok(_) => Ok(false),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

#[cfg(not(unix))]
fn is_executable(p: &Path) -> std::io::Result<bool> {
    Ok(p.is_file())
}

/// Global options that take their value as a *separate* argument, so the
/// value must not be mistaken for the subcommand name. Keep in sync with
/// the `argument(…)` options on `FloxArgs`; the `--flag=value` spelling
/// needs no entry because it is a single `-`-prefixed token.
const GLOBAL_VALUE_OPTIONS: &[&str] = &["--floxhub-url"];

/// `argv` (minus `argv[0]`) split at the subcommand name.
struct Split {
    /// The first non-flag argument, i.e. the candidate extension name.
    name: Option<OsString>,
    /// Everything after the name, forwarded to the extension verbatim.
    rest: Vec<OsString>,
}

/// Split `flox`'s own arguments into the subcommand name and the
/// extension's arguments, discarding the global flags that precede the
/// name.
///
/// Those globals were not applied to flox itself — dispatch only runs on a
/// parse failure — and are not forwarded. A flag placed *after* the name
/// (`flox myext -v`) belongs to the extension and lands in `rest`.
fn split_argv(argv: impl IntoIterator<Item = OsString>) -> Split {
    let mut iter = argv.into_iter();
    let mut name = None;
    while let Some(arg) = iter.next() {
        if arg.as_encoded_bytes().first() != Some(&b'-') {
            name = Some(arg);
            break;
        }
        // Drop the value along with its option so it can't be taken for
        // the name.
        if GLOBAL_VALUE_OPTIONS.iter().any(|o| arg == *o) {
            iter.next();
        }
    }
    Split {
        name,
        rest: iter.collect(),
    }
}

/// Two-phase parse fallback: when the top-level bpaf parse fails (because
/// `flox <name>` doesn't match a known subcommand), check whether `<name>`
/// resolves to a managed or PATH-installed `flox-<name>` and exec it.
///
/// Returns `Some(exit_code)` when the dispatch happens (success or failure
/// of the child process), or `None` when no extension matched and the
/// caller should fall back to its existing parse-error path.
///
/// Extensions are a beta feature and are **off** by default. The caller
/// gates this behind the beta check in `main()`, mirroring how the
/// `Commands::Beta` arm gates the `flox extension …` subcommands — beta
/// code does not re-check the flag. That gate reads the config only, so
/// unlike the beta subcommands, dispatch is not enabled by the ephemeral
/// `--beta` flag: reading it back out of argv here would be the only
/// consumer of a flag the parser has already rejected. `features.beta`
/// and `FLOX_FEATURES_BETA` are the two ways in.
///
/// `data_dir` is the caller's resolved `flox.data_dir`; the managed
/// extensions live under `data_dir/extensions`, matching
/// [`super::layout::extensions_root`].
pub fn try_dispatch_external(data_dir: &Path) -> Option<ExitCode> {
    let Split { name, rest, .. } = split_argv(std::env::args_os().skip(1));
    let name = name?;
    let name_str = name.to_str()?;

    // Never let the external fallback shadow a built-in command. If the
    // first token names a reserved (built-in) command, the parse failure
    // was a bad invocation of that command — e.g. `flox init --badflag` —
    // not an extension. Fall through to the parser's error instead of
    // exec'ing a `flox-init` that happens to be on $PATH.
    if super::RESERVED_COMMAND_NAMES
        .iter()
        .any(|r| r.eq_ignore_ascii_case(name_str))
    {
        return None;
    }

    let extensions_root = data_dir.join("extensions");
    let path_env = std::env::var_os("PATH");
    let path = match find(name_str, &extensions_root, path_env.as_deref()) {
        Ok(p) => p,
        Err(FindError::NotFound(_)) => return None,
        Err(e) => {
            warn!(extension = name_str, error = %e, "extension lookup failed");
            return None;
        },
    };

    debug!(extension = name_str, path = ?path, "dispatching to external extension");

    let install_dir = managed_install_dir(&path, &extensions_root);
    let installed_state = install_dir.as_deref().and_then(load_installed_state);

    let extension_name = installed_state
        .as_ref()
        .map(|s| s.name.clone())
        .unwrap_or_else(|| name_str.to_string());
    let extension_path = install_dir
        .as_ref()
        .map(|p| p.as_os_str().to_owned())
        .unwrap_or_else(|| path.as_os_str().to_owned());
    let flox_bin = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("flox"));

    let mut command = Command::new(&path);
    command
        .args(&rest)
        .env("FLOX_EXTENSION_NAME", &extension_name)
        .env("FLOX_EXTENSION_PATH", &extension_path)
        .env("FLOX_BIN", &flox_bin);

    let err = replace_process(&mut command);
    eprintln!("flox: failed to execute '{}': {}", path.display(), err);
    Some(ExitCode::from(1))
}

/// Wrapper around `<Command as CommandExt>::exec` — replaces the current
/// process in place via the `execvp(2)` syscall. Never returns on success.
/// Args are passed as a separate vector; no shell is spawned.
fn replace_process(command: &mut Command) -> std::io::Error {
    <Command as CommandExt>::exec(command)
}

/// The install directory for a managed extension, if `exe_path` lives
/// under `extensions_root` in the expected `<root>/<flox-name>/<flox-name>`
/// layout. Returns `None` for PATH-fallback extensions (which have no
/// managed install_dir) or when `exe_path` is in an unexpected shape.
fn managed_install_dir(exe_path: &Path, extensions_root: &Path) -> Option<PathBuf> {
    let parent = exe_path.parent()?;
    if !parent.starts_with(extensions_root) {
        return None;
    }
    Some(parent.to_path_buf())
}

fn load_installed_state(install_dir: &Path) -> Option<InstalledState> {
    let path = install_dir.join("state.toml");
    let contents = std::fs::read_to_string(&path).ok()?;
    match parse_installed_state(&contents) {
        Ok(s) => Some(s),
        Err(e) => {
            warn!(path = %path.display(), error = %e, "failed to parse extension installed state");
            None
        },
    }
}

#[cfg(test)]
#[cfg(feature = "beta-tests")]
mod tests {
    use std::ffi::OsString;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use tempfile::TempDir;

    use super::*;

    /// Create `extensions_root/flox-<name>/flox-<name>` as an executable
    /// file. Returns the full path.
    fn mk_managed_ext(extensions_root: &Path, name: &str) -> PathBuf {
        let exe_name = format!("flox-{name}");
        let dir = extensions_root.join(&exe_name);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(&exe_name);
        fs::write(&path, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    /// Create `path_dir/flox-<name>` as an executable file. Returns the
    /// full path.
    fn mk_path_ext(path_dir: &Path, name: &str) -> PathBuf {
        let path = path_dir.join(format!("flox-{name}"));
        fs::write(&path, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn find_returns_err_for_missing_name() {
        let managed = TempDir::new().unwrap();
        let path_dir = TempDir::new().unwrap();
        let path_env = OsString::from(path_dir.path());

        let err = find("foo", managed.path(), Some(&path_env)).unwrap_err();
        assert!(matches!(err, FindError::NotFound(ref n) if n == "foo"));
    }

    #[test]
    fn find_picks_managed_dir_over_path() {
        let managed = TempDir::new().unwrap();
        let path_dir = TempDir::new().unwrap();
        let managed_path = mk_managed_ext(managed.path(), "foo");
        let _path_path = mk_path_ext(path_dir.path(), "foo");
        let path_env = OsString::from(path_dir.path());

        let got = find("foo", managed.path(), Some(&path_env)).unwrap();
        assert_eq!(got, managed_path);
    }

    #[test]
    fn find_falls_back_to_path() {
        let managed = TempDir::new().unwrap();
        let path_dir = TempDir::new().unwrap();
        let path_path = mk_path_ext(path_dir.path(), "foo");
        let path_env = OsString::from(path_dir.path());

        let got = find("foo", managed.path(), Some(&path_env)).unwrap();
        assert_eq!(got, path_path);
    }

    #[cfg(unix)]
    #[test]
    fn find_rejects_non_executable() {
        let managed = TempDir::new().unwrap();
        let exe_name = "flox-foo";
        let dir = managed.path().join(exe_name);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(exe_name);
        fs::write(&path, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

        let err = find("foo", managed.path(), None).unwrap_err();
        assert!(matches!(err, FindError::NotFound(ref n) if n == "foo"));
    }

    #[test]
    fn find_rejects_names_with_slashes() {
        let managed = TempDir::new().unwrap();
        let err = find("../etc/passwd", managed.path(), None).unwrap_err();
        assert!(matches!(err, FindError::NotFound(ref n) if n == "../etc/passwd"));
    }

    fn split(args: &[&str]) -> (Option<String>, Vec<String>) {
        let split = split_argv(args.iter().map(OsString::from));
        (
            split.name.map(|n| n.to_string_lossy().into_owned()),
            split
                .rest
                .iter()
                .map(|a| a.to_string_lossy().into_owned())
                .collect(),
        )
    }

    #[test]
    fn split_argv_takes_first_non_flag_as_name() {
        assert_eq!(
            split(&["-v", "myext", "-v", "arg"]),
            (Some("myext".to_string()), vec![
                "-v".to_string(),
                "arg".to_string()
            ],)
        );
    }

    #[test]
    fn split_argv_skips_global_option_values() {
        assert_eq!(
            split(&["--floxhub-url", "https://example.com", "myext"]),
            (Some("myext".to_string()), vec![])
        );
        assert_eq!(
            split(&["--floxhub-url=https://example.com", "myext"]),
            (Some("myext".to_string()), vec![],)
        );
    }

    #[test]
    fn split_argv_without_a_name() {
        assert_eq!(split(&["--beta"]), (None, vec![]));
    }

    #[test]
    fn find_rejects_empty_name() {
        let managed = TempDir::new().unwrap();
        let err = find("", managed.path(), None).unwrap_err();
        assert!(matches!(err, FindError::NotFound(ref n) if n.is_empty()));
    }
}
