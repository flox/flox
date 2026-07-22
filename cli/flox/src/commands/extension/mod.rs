//! `flox extension` subcommand group.

use std::ffi::OsString;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anyhow::Result;
use beta::extensions::dispatch::{
    self,
    ActivationMode,
    DispatchError,
    resolve_mode,
    scrub_flox_env,
};
use beta::extensions::{
    AuthorManifest,
    InstalledState,
    parse_author_manifest,
    parse_installed_state,
};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use tracing::{debug, warn};

use crate::utils::active_environments::activated_environments;

mod format;
mod install;
mod list;
mod remove;
mod search;
mod upgrade;

/// Manage flox extensions
#[derive(Debug, Bpaf, Clone)]
pub enum ExtensionCommands {
    /// Install an extension
    #[bpaf(command)]
    Install(#[bpaf(external(install::install))] install::Install),

    /// List installed extensions
    #[bpaf(command)]
    List(#[bpaf(external(list::list))] list::List),

    /// Remove an installed extension
    #[bpaf(command)]
    Remove(#[bpaf(external(remove::remove))] remove::Remove),

    /// Search GitHub for flox extensions
    #[bpaf(command)]
    Search(#[bpaf(external(search::search))] search::Search),

    /// Upgrade one or all installed extensions
    #[bpaf(command)]
    Upgrade(#[bpaf(external(upgrade::upgrade))] upgrade::Upgrade),
}

impl ExtensionCommands {
    pub async fn handle(self, flox: Flox) -> Result<()> {
        match self {
            ExtensionCommands::Install(args) => args.handle(flox).await,
            ExtensionCommands::List(args) => args.handle(flox).await,
            ExtensionCommands::Remove(args) => args.handle(flox).await,
            ExtensionCommands::Search(args) => args.handle(flox).await,
            ExtensionCommands::Upgrade(args) => args.handle(flox).await,
        }
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
pub fn try_dispatch_external() -> Option<ExitCode> {
    // TODO(flox/flox#4537): dispatch runs before config loads, so it reads
    // FLOX_FEATURES_BETA and reconstructs the extensions root from XDG
    // instead of the resolved `features.beta` / `flox.data_dir`. This
    // diverges from the `flox extension …` subcommands (config-enabled beta
    // and a config-set data_dir are not honored here). Fix deferred; load
    // the effective config lazily on this path. See the issue for repros.
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
    if beta::extensions::RESERVED_COMMAND_NAMES
        .iter()
        .any(|r| r.eq_ignore_ascii_case(name_str))
    {
        return None;
    }

    let extensions_root = extensions_root();
    let path_env = std::env::var_os("PATH");
    let path = match dispatch::find(name_str, &extensions_root, path_env.as_deref()) {
        Ok(p) => p,
        Err(dispatch::FindError::NotFound(_)) => return None,
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
fn ref_matches_active(
    pinned_ref: &str,
    active: &crate::utils::active_environments::ActiveEnvironments,
) -> bool {
    let Some((owner, name)) = pinned_ref.split_once('/') else {
        return false;
    };
    active.iter().any(|env| {
        env.owner_if_managed().map(|o| o.as_str()) == Some(owner) && env.name().as_ref() == name
    })
}

/// Whether beta features are enabled, according to the environment alone.
///
/// The `Commands::Beta` arm gates every beta subcommand on
/// [`Flox::features`] before dispatching, so the `flox extension …`
/// handlers must not re-check. This exists solely for
/// [`try_dispatch_external`], which runs before `Flox` is initialized —
/// see the limitation documented there.
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
/// This must agree with `beta::extensions::layout::extensions_root`,
/// which derives from `flox.data_dir`. If they diverge, `flox extension install`
/// writes to one path and `flox <name>` looks in another.
fn extensions_root() -> PathBuf {
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
