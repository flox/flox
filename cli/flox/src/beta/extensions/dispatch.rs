//! External-subcommand resolution and dispatch.
//!
//! [`try_dispatch_external`] is the whole `flox <name>` fallback: resolve
//! `<name>` to a `flox-<name>` executable, decide what environment it
//! should see, and replace the current process with it.
//!
//! Resolution ([`find`]) searches the managed extensions directory first,
//! then `$PATH`. The managed-dir-first order is intentional: the managed
//! dir is populated only by `flox extension install`, so precedence there
//! reflects explicit user intent rather than ambient shell state.
//!
//! [`resolve_mode`] maps the author manifest's `[environment]` stanza to
//! the [`ActivationMode`] to apply, and [`scrub_flox_env`] implements
//! None-mode env scrubbing.

use std::ffi::{OsStr, OsString};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use thiserror::Error;
use tracing::{debug, warn};

use super::manifest::{
    AuthorManifest,
    EnvironmentBehavior,
    InstalledState,
    parse_author_manifest,
    parse_installed_state,
};
use crate::utils::active_environments::{ActiveEnvironments, activated_environments};

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

/// Two-phase parse fallback: when the top-level bpaf parse fails (because
/// `flox <name>` doesn't match a known subcommand), check whether `<name>`
/// resolves to a managed or PATH-installed `flox-<name>` and exec it.
///
/// Returns `Some(exit_code)` when the dispatch happens (success or failure
/// of the child process), or `None` when no extension matched and the
/// caller should fall back to its existing parse-error path.
///
/// Extensions are a beta feature and are **off** by default. Dispatch only
/// fires when `FLOX_FEATURES_BETA` is set to `true` (or `1`) in the
/// environment.
///
/// # Known limitation
///
/// This reads the environment variable directly rather than consulting
/// [`Flox::features`], because `Flox` is not yet initialized at the
/// parse-failure point in `main()` where this is called. Consequently
/// `flox config --set features.beta true` — which writes the config file
/// and does *not* set the environment variable — enables the
/// `flox extension …` subcommands but **not** `flox <name>` dispatch.
/// Users who enable beta via config must also export
/// `FLOX_FEATURES_BETA=true` for dispatch to work. This is documented in
/// the user guide.
///
/// [`Flox::features`]: flox_rust_sdk::flox::Flox::features
pub fn try_dispatch_external() -> Option<ExitCode> {
    // TODO(CLI-158): dispatch runs before config loads, so it reads
    // FLOX_FEATURES_BETA and reconstructs the extensions root from XDG
    // instead of the resolved `features.beta` / `flox.data_dir`. This
    // diverges from the `flox extension …` subcommands (config-enabled beta
    // and a config-set data_dir are not honored here). Fix deferred; load
    // the effective config lazily on this path. See the issue for repros.
    // https://linear.app/floxdotdev/issue/CLI-158
    if !beta_enabled_from_env() {
        return None;
    }

    let mut argv: Vec<OsString> = std::env::args_os().collect();
    if argv.is_empty() {
        return None;
    }
    argv.remove(0);

    // Skip leading global flags (e.g. `flox -v myext`). They were not
    // applied to flox itself — we only reach this path on parse failure —
    // and are not forwarded to the extension. A flag placed after the name
    // (`flox myext -v`) falls into `rest` and reaches the child.
    let mut iter = argv.into_iter();
    let mut name: Option<OsString> = None;
    for arg in iter.by_ref() {
        if arg.as_encoded_bytes().first() == Some(&b'-') {
            continue;
        }
        name = Some(arg);
        break;
    }
    let rest: Vec<OsString> = iter.collect();
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

    let extensions_root = extensions_root_from_env();
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
    let author_manifest = match install_dir.as_deref().map(load_author_manifest) {
        None => None,
        Some(Ok(m)) => m,
        Some(Err(msg)) => {
            // Fail closed: a present-but-unreadable manifest may declare a
            // restrictive activation policy we must not silently ignore.
            eprintln!("flox: {msg}");
            return Some(ExitCode::from(1));
        },
    };
    let installed_state = install_dir.as_deref().and_then(load_installed_state);

    let mode = resolve_mode(
        author_manifest
            .as_ref()
            .and_then(|m| m.environment.as_ref()),
    );
    let on_active_inside = author_manifest
        .as_ref()
        .and_then(|m| m.on_active.as_ref())
        .map(|o| o.inside.as_str())
        .unwrap_or_default();

    let extension_name = installed_state
        .as_ref()
        .map(|s| s.name.clone())
        .unwrap_or_else(|| name_str.to_string());
    let extension_version = installed_state
        .as_ref()
        .map(version_from_state)
        .unwrap_or_else(|| "-".to_string());
    let extension_path = install_dir
        .as_ref()
        .map(|p| p.as_os_str().to_owned())
        .unwrap_or_else(|| path.as_os_str().to_owned());
    let flox_bin = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("flox"));

    let mut command = match build_dispatch_command(
        &mode,
        &path,
        &rest,
        &flox_bin,
        &extension_name,
        on_active_inside,
    ) {
        Ok(cmd) => cmd,
        Err(e) => {
            eprintln!("flox: {e}");
            return Some(ExitCode::from(1));
        },
    };
    command
        .env("FLOX_EXTENSION_NAME", &extension_name)
        .env("FLOX_EXTENSION_VERSION", &extension_version)
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

/// Load the author manifest for dispatch.
///
/// `Ok(None)` means no manifest is present (no declared policy → the
/// caller's default applies). `Err` means a manifest *is* present but
/// could not be read or parsed: dispatch must fail closed rather than
/// silently defaulting to `Inherit`, because the unreadable manifest may
/// have declared `mode = "none"` (a scrubbed environment) or a pinned
/// environment.
fn load_author_manifest(install_dir: &Path) -> Result<Option<AuthorManifest>, String> {
    let path = install_dir.join("flox-extension.toml");
    if !path.exists() {
        return Ok(None);
    }
    let contents = std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read extension manifest {}: {e}", path.display()))?;
    parse_author_manifest(&contents)
        .map(Some)
        .map_err(|e| format!("invalid extension manifest {}: {e}", path.display()))
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

fn version_from_state(state: &InstalledState) -> String {
    if !state.tag.is_empty() {
        return state.tag.clone();
    }
    // Truncate by characters, not bytes: a byte slice at offset 8 can land
    // inside a multibyte codepoint and panic on a non-ASCII (corrupt)
    // commit value.
    let short: String = state.commit.chars().take(8).collect();
    if short.chars().count() == 8 {
        return short;
    }
    "-".to_string()
}

/// Build the `Command` for a given activation mode. Does not inject the
/// `FLOX_EXTENSION_*` bookkeeping vars; the caller overlays those on all
/// three modes.
fn build_dispatch_command(
    mode: &ActivationMode,
    ext_path: &Path,
    rest: &[OsString],
    flox_bin: &Path,
    extension_name: &str,
    on_active_inside: &str,
) -> Result<Command, DispatchError> {
    match mode {
        ActivationMode::Inherit => {
            let mut cmd = Command::new(ext_path);
            cmd.args(rest);
            Ok(cmd)
        },
        ActivationMode::None => {
            let mut cmd = Command::new(ext_path);
            cmd.args(rest);
            cmd.env_clear();
            cmd.envs(scrub_flox_env(std::env::vars_os()));
            Ok(cmd)
        },
        ActivationMode::Pinned(pinned_ref) => {
            let active = activated_environments();
            if ref_matches_active(pinned_ref, &active) {
                let mut cmd = Command::new(ext_path);
                cmd.args(rest);
                return Ok(cmd);
            }
            if on_active_inside == "error" && active.iter().next().is_some() {
                return Err(DispatchError::PinnedEnvMismatch {
                    extension: extension_name.to_string(),
                    expected: pinned_ref.clone(),
                });
            }
            let mut cmd = Command::new(flox_bin);
            cmd.arg("activate").arg("-r").arg(pinned_ref).arg("--");
            cmd.arg(ext_path);
            cmd.args(rest);
            Ok(cmd)
        },
    }
}

/// Return true when the caller is already activated in the environment
/// referenced by `pinned_ref` (an opaque `owner/name` string). Non-matching
/// or malformed refs degrade to `false` (the caller will wrap with `flox
/// activate -r`).
fn ref_matches_active(pinned_ref: &str, active: &ActiveEnvironments) -> bool {
    let Some((owner, name)) = pinned_ref.split_once('/') else {
        return false;
    };
    active.iter().any(|env| {
        env.owner_if_managed().map(|o| o.as_str()) == Some(owner) && env.name().as_ref() == name
    })
}

/// Whether beta features are enabled, according to the environment alone.
///
/// The `Commands::Beta` arm gates every beta subcommand on `flox.features`
/// before dispatching, so the `flox extension …` handlers must not
/// re-check. This exists solely for [`try_dispatch_external`], which runs
/// before `Flox` is initialized — see the limitation documented there.
///
/// Accepts the spelling the docs use (`true`) plus `1`. Anything else,
/// including unset, leaves the feature off.
fn beta_enabled_from_env() -> bool {
    matches!(
        std::env::var("FLOX_FEATURES_BETA")
            .ok()
            .as_deref()
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("true") | Some("1")
    )
}

/// Compute the extensions root the same way `flox.data_dir` resolves:
///   1. `FLOX_DATA_DIR` env var (matches the config-system override that
///      `Flox::data_dir` would pick up after `Flox::init`), then
///   2. `XDG_DATA_HOME/flox`, then
///   3. `$HOME/.local/share/flox`.
///
/// Empty-string values are treated as unset, matching the `xdg` crate used
/// by `BaseDirectories::with_prefix("flox")` in the config system.
///
/// This must agree with [`super::layout::extensions_root`], which derives
/// from `flox.data_dir`. If they diverge, `flox extension install` writes
/// to one path and `flox <name>` looks in another.
fn extensions_root_from_env() -> PathBuf {
    if let Some(d) = non_empty_env("FLOX_DATA_DIR") {
        return PathBuf::from(d).join("extensions");
    }
    let base = non_empty_env("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| non_empty_env("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("flox").join("extensions")
}

fn non_empty_env(key: &str) -> Option<OsString> {
    std::env::var_os(key).filter(|v| !v.is_empty())
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
