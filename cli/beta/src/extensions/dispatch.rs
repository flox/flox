//! External-subcommand resolver.
//!
//! Given a subcommand name, locate the executable `flox-<name>` that
//! implements it. Searches the managed extensions directory first, then
//! `$PATH`. The managed-dir-first order is intentional: the managed dir
//! is populated only by `flox extension install`, so precedence there
//! reflects explicit user intent rather than ambient shell state.
//!
//! P01 introduced [`find`]. P06 adds [`ActivationMode`] + [`resolve_mode`]
//! (pure mapping from the author manifest's `[environment]` stanza to
//! the mode the dispatch layer should apply) and [`scrub_flox_env`]
//! (helper for None-mode env scrubbing). Process replacement and
//! bookkeeping-var injection live on the CLI side next to
//! `try_dispatch_external`.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use thiserror::Error;
use tracing::warn;

use super::manifest::EnvironmentBehavior;

#[derive(Debug, Error)]
pub enum FindError {
    #[error("no extension named '{0}' is installed")]
    NotFound(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Dispatch-time activation-mode selection derived from the author
/// manifest's `[environment]` stanza. The CLI layer maps this to a
/// `Command` before process replacement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActivationMode {
    /// Launch in the caller's current environment (possibly none).
    Inherit,
    /// Scrub `FLOX_*` / `_FLOX_*` before launch; no activation wrapper.
    None,
    /// Re-invoke `flox activate -r <ref> -- <ext>`. Opaque owner/name.
    Pinned(String),
}

/// P06 dispatch-side errors raised before or during process replacement.
#[derive(Debug, Error)]
pub enum DispatchError {
    #[error(
        "extension '{extension}' requires the '{expected}' environment; trust it with 'flox activate -r {expected} --trust' first"
    )]
    PinnedEnvMismatch { extension: String, expected: String },
}

/// Map an author-manifest `[environment]` stanza to the mode the dispatch
/// layer should apply.
///
/// Rules (research-doc §1.11):
/// - missing stanza → `Inherit`
/// - `mode = "none"` → `None`
/// - `mode = "inherit"` (or empty) → `Inherit`
/// - `mode = "pinned"` with non-empty `inherit_name` → `Pinned(ref)`
/// - `mode = "pinned"` with missing/empty `inherit_name` → warn, fall
///   back to `Inherit` (manifest is malformed; don't hard-fail dispatch)
/// - any other value → warn, fall back to `Inherit` (lenient
///   forward-compat)
///
/// Idempotency (when the caller is already inside the pinned env) is
/// handled on the CLI side after this function returns, using
/// `_FLOX_ACTIVE_ENVIRONMENTS`. Keeping that check out of the SDK avoids
/// duplicating the `ActiveEnvironments` JSON parser.
pub fn resolve_mode(manifest_env: Option<&EnvironmentBehavior>) -> ActivationMode {
    let Some(env) = manifest_env else {
        return ActivationMode::Inherit;
    };
    match env.mode.as_str() {
        "none" => ActivationMode::None,
        "inherit" | "" => ActivationMode::Inherit,
        "pinned" => match env.inherit_name.as_deref() {
            Some(name) if !name.is_empty() => ActivationMode::Pinned(name.to_owned()),
            _ => {
                warn!(
                    mode = "pinned",
                    "extension manifest: pinned mode requires non-empty inherit_name; falling back to Inherit"
                );
                ActivationMode::Inherit
            },
        },
        other => {
            warn!(
                mode = other,
                "extension manifest: unknown environment.mode; falling back to Inherit"
            );
            ActivationMode::Inherit
        },
    }
}

/// Filter `env_vars` down to the set safe to pass in `ActivationMode::None`.
///
/// Drops every key whose byte prefix is `FLOX_` or `_FLOX_`. `FLOXHUB_*`
/// is intentionally preserved: it is not a flox-activation-context
/// variable, and None-mode is about hiding the enclosing flox
/// environment, not about scrubbing FloxHub credentials.
pub fn scrub_flox_env(
    env_vars: impl IntoIterator<Item = (OsString, OsString)>,
) -> Vec<(OsString, OsString)> {
    env_vars
        .into_iter()
        .filter(|(key, _)| !is_flox_prefixed(key))
        .collect()
}

fn is_flox_prefixed(key: &OsStr) -> bool {
    let bytes = key.as_encoded_bytes();
    bytes.starts_with(b"FLOX_") || bytes.starts_with(b"_FLOX_")
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

#[cfg(test)]
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

    #[test]
    fn find_rejects_empty_name() {
        let managed = TempDir::new().unwrap();
        let err = find("", managed.path(), None).unwrap_err();
        assert!(matches!(err, FindError::NotFound(ref n) if n.is_empty()));
    }

    fn env(mode: &str, inherit_name: Option<&str>) -> EnvironmentBehavior {
        EnvironmentBehavior {
            mode: mode.to_string(),
            inherit: None,
            inherit_name: inherit_name.map(str::to_string),
        }
    }

    #[test]
    fn resolve_mode_none_manifest_returns_inherit() {
        assert_eq!(resolve_mode(None), ActivationMode::Inherit);
    }

    #[test]
    fn resolve_mode_inherit_returns_inherit() {
        assert_eq!(
            resolve_mode(Some(&env("inherit", None))),
            ActivationMode::Inherit
        );
    }

    #[test]
    fn resolve_mode_empty_mode_returns_inherit() {
        assert_eq!(resolve_mode(Some(&env("", None))), ActivationMode::Inherit);
    }

    #[test]
    fn resolve_mode_none_returns_none() {
        assert_eq!(resolve_mode(Some(&env("none", None))), ActivationMode::None);
    }

    #[test]
    fn resolve_mode_pinned_with_name_returns_pinned() {
        assert_eq!(
            resolve_mode(Some(&env("pinned", Some("alice/proj")))),
            ActivationMode::Pinned("alice/proj".to_string())
        );
    }

    #[test]
    fn resolve_mode_pinned_without_name_falls_back_to_inherit() {
        assert_eq!(
            resolve_mode(Some(&env("pinned", None))),
            ActivationMode::Inherit
        );
    }

    #[test]
    fn resolve_mode_pinned_with_empty_name_falls_back_to_inherit() {
        assert_eq!(
            resolve_mode(Some(&env("pinned", Some("")))),
            ActivationMode::Inherit
        );
    }

    #[test]
    fn resolve_mode_unknown_mode_falls_back_to_inherit() {
        assert_eq!(
            resolve_mode(Some(&env("frobnicate", None))),
            ActivationMode::Inherit
        );
    }

    fn os(s: &str) -> OsString {
        OsString::from(s)
    }

    #[test]
    fn scrub_flox_env_removes_flox_and_underscore_flox_prefixes() {
        let input = vec![
            (os("FLOX_ENV"), os("/some/env")),
            (os("FLOX_PROMPT"), os("foo")),
            (os("_FLOX_ACTIVE_ENVIRONMENTS"), os("[]")),
            (os("PATH"), os("/usr/bin")),
            (os("HOME"), os("/home/u")),
            (os("FLOXHUB_TOKEN"), os("secret")),
        ];
        let out = scrub_flox_env(input);
        let keys: Vec<OsString> = out.iter().map(|(k, _)| k.clone()).collect();
        assert_eq!(keys, vec![os("PATH"), os("HOME"), os("FLOXHUB_TOKEN"),]);
    }

    #[test]
    fn scrub_flox_env_on_empty_input_returns_empty() {
        let out: Vec<(OsString, OsString)> = scrub_flox_env(Vec::new());
        assert!(out.is_empty());
    }

    #[test]
    fn scrub_flox_env_preserves_non_flox_keys_including_floxhub() {
        let input = vec![
            (os("FLOXHUB_TOKEN"), os("secret")),
            (os("FLOXHUB_URL"), os("https://hub.flox.dev")),
            (os("FLOOR"), os("tile")),
            (os("FLOX"), os("literal")),
        ];
        let out = scrub_flox_env(input.clone());
        assert_eq!(out, input);
    }
}
