//! Session-wrap plugin hook dispatch.
//!
//! A plugin declared under `[plugin-hooks].session-wrap` re-enters the
//! activation under an enforcement boundary of its choosing: after the
//! environment is locked, built, and rendered — hooks are discovered in the
//! rendered `$FLOX_ENV`, so render-before-wrap is a hard requirement — the
//! CLI execs the plugin's hook executable, which never returns on success.
//! The hook receives a serialized [`SessionWrapCtx`] and either re-execs the
//! host-side `inner_argv` under a host boundary (same-filesystem wrappers)
//! or composes its own in-boundary command from `invocation_type`
//! (container and remote wrappers).
//!
//! Re-entry is cooperative: the hook exports [`SESSION_WRAPPED_VAR`] set to
//! the ctx's `wrap_scope` on the wrapped process, and dispatch is skipped
//! only when the marker matches the environment being activated. A mismatch
//! means an activation of a *different* wrapping environment inside an
//! existing boundary, which is a nested-boundary error rather than a silent
//! unwrapped activation. The marker is re-entry detection, not boundary
//! integrity — integrity must come from the boundary itself.
//!
//! Design: docs/plugin-lifecycle-hooks.md.

use std::convert::Infallible;
use std::io::{BufWriter, IsTerminal};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use anyhow::{Context, Result, bail};
use flox_core::activate::context::InvocationType;
use flox_core::path_hash;
use flox_manifest::lockfile::{LockedPackage, Lockfile};
use flox_manifest::parsed::Inner;
use flox_manifest::parsed::latest::ManifestLatest;
use indoc::formatdoc;
use serde::Serialize;
use tracing::debug;

use crate::utils::message;

/// Environment variable holding the path to the serialized [`SessionWrapCtx`].
pub const FLOX_HOOK_CTX_VAR: &str = "FLOX_HOOK_CTX";
/// Environment variable naming the hook being invoked (`session-wrap`).
pub const FLOX_HOOK_VAR: &str = "FLOX_HOOK";
/// Environment variable naming the plugin whose hook is invoked.
pub const FLOX_PLUGIN_NAME_VAR: &str = "FLOX_PLUGIN_NAME";
/// Environment variable pointing at the invoking flox binary.
pub const FLOX_BIN_VAR: &str = "FLOX_BIN";
/// Environment variable pointing at a jq the hook may rely on for parsing
/// its ctx, so shell-scripted hooks need not depend on one themselves.
pub const FLOX_HOOK_JQ_VAR: &str = "FLOX_HOOK_JQ";

/// Marker exported by a session-wrap hook on the wrapped (inner) process.
///
/// Its value is the ctx's `wrap_scope`; dispatch is skipped on a match and
/// errors on a mismatch. Living here rather than in plugin code makes the
/// cross-plugin protocol visible in one place.
pub const SESSION_WRAPPED_VAR: &str = "_FLOX_SESSION_WRAPPED";

/// Hook directory inside the rendered environment, relative to `$FLOX_ENV`.
const SESSION_WRAP_HOOK_DIR: &str = "etc/flox/hooks/session-wrap.d";

/// jq bundled at build time for hook consumption (`FLOX_HOOK_JQ`), following
/// the `PROCESS_COMPOSE_BIN` pattern of using our own binaries by absolute
/// path rather than relying on the user's `PATH`.
static JQ_BIN: LazyLock<String> =
    LazyLock::new(|| std::env::var("JQ_BIN").unwrap_or(env!("JQ_BIN").to_string()));

/// Everything a session-wrap hook needs to re-enter the activation under its
/// boundary. Serialized as JSON to a `0600` file whose path is passed via
/// [`FLOX_HOOK_CTX_VAR`]; the schema is versioned via `ctx_version`.
#[derive(Debug, Clone, Serialize)]
pub struct SessionWrapCtx {
    /// Version of this context schema. Bump on breaking shape changes.
    pub ctx_version: u32,
    /// Absolute path to the `.flox` directory of the environment.
    pub dot_flox_path: PathBuf,
    /// Short environment name, for image tags and messages.
    pub env_name: String,
    /// Activation mode (`dev` or `run`).
    pub activation_mode: String,
    /// Store path of the rendered environment for the activation mode.
    pub rendered_env: PathBuf,
    /// Path to the environment's lockfile. Its schema is published at
    /// `cli/schemas/lockfile-v1.schema.json`.
    pub lockfile_path: PathBuf,
    /// The plugin's own `[plugins.<name>]` table, verbatim. `null` when the
    /// manifest carries no table for the plugin.
    pub plugin_table: serde_json::Value,
    /// How the user invoked `flox activate`, with its full payload (the
    /// command vector for `-- <cmd>`, the shell string for `-c`). Container
    /// and remote wrappers compose their in-boundary command from this.
    pub invocation_type: InvocationType,
    /// Whether stdin/stdout are terminals. Stdio is inherited, not
    /// guaranteed a tty: a hook that prompts must write to stderr or
    /// `/dev/tty`, never stdout.
    pub stdin_is_tty: bool,
    pub stdout_is_tty: bool,
    /// Host-side re-entry argv: the invocation to re-exec under a host
    /// boundary. Only meaningful for wrappers that share the host
    /// filesystem; container wrappers must use `invocation_type` instead.
    pub inner_argv: Vec<String>,
    /// Value the hook must export as [`SESSION_WRAPPED_VAR`] on the wrapped
    /// process so re-entry skips dispatch.
    pub wrap_scope: String,
}

/// A resolved, validated session-wrap dispatch, ready to exec.
#[derive(Debug)]
pub struct SessionWrapExec {
    hook_path: PathBuf,
    plugin_name: String,
    ctx: SessionWrapCtx,
}

/// Outcome of session-wrap resolution for this activation.
#[derive(Debug)]
pub enum SessionWrap {
    /// No wrapping: nothing declared, the feature is off, re-entry, or an
    /// ephemeral activation.
    NoWrap,
    /// Exec the hook; never returns on success.
    Wrap(Box<SessionWrapExec>),
}

/// Inputs to [`resolve`] that activate.rs already has in scope.
pub struct SessionWrapArgs<'a> {
    pub manifest: &'a ManifestLatest,
    pub lockfile: &'a Lockfile,
    pub lockfile_path: PathBuf,
    pub dot_flox_path: PathBuf,
    pub env_name: String,
    pub activation_mode: String,
    pub rendered_env: PathBuf,
    pub system: &'a str,
    pub invocation_type: &'a InvocationType,
    pub is_ephemeral: bool,
    pub feature_enabled: bool,
}

/// The scope value identifying "this environment, wrapped by this plugin".
fn wrap_scope(dot_flox_path: &Path, plugin_name: &str) -> String {
    path_hash(dot_flox_path.join(plugin_name))
}

/// Store paths provided by a locked package, used to verify that the hook
/// file the rendered environment links actually comes from the declared
/// plugin's package rather than another install shadowing its filename.
fn package_store_paths(package: &LockedPackage) -> Vec<PathBuf> {
    match package {
        LockedPackage::Catalog(pkg) => pkg.outputs.values().map(PathBuf::from).collect(),
        LockedPackage::Flake(pkg) => pkg
            .locked_installable
            .outputs
            .values()
            .map(PathBuf::from)
            .collect(),
        LockedPackage::StorePath(pkg) => vec![PathBuf::from(&pkg.store_path)],
    }
}

/// Warn about session-wrap hook files shipped by installed packages that are
/// not armed by a `[plugin-hooks]` declaration. Ignoring them is what makes
/// declarations meaningful; saying so keeps it from looking like breakage.
fn warn_undeclared_hook_files(hook_dir: &Path, declared: Option<&str>) {
    let Ok(entries) = std::fs::read_dir(hook_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if Some(name.as_ref()) != declared {
            message::warning(formatdoc! {"
                Ignored session-wrap hook '{name}' shipped by an installed package.
                Declare it under [plugin-hooks] in the manifest to enable it."});
        }
    }
}

/// Decide whether this activation is wrapped, enforcing the declaration
/// cross-checks. See the module docs for the rules' rationale.
pub fn resolve(args: SessionWrapArgs<'_>) -> Result<SessionWrap> {
    let declared = args
        .manifest
        .plugin_hooks
        .as_ref()
        .and_then(|hooks| hooks.session_wrap.as_deref());

    if !args.feature_enabled {
        if declared.is_some() {
            message::warning(formatdoc! {"
                Ignored [plugin-hooks] because the 'plugin_hooks' feature is not enabled.
                Enable it with 'flox config --set features.plugin_hooks true'."});
        }
        return Ok(SessionWrap::NoWrap);
    }

    let hook_dir = args.rendered_env.join(SESSION_WRAP_HOOK_DIR);
    warn_undeclared_hook_files(&hook_dir, declared);

    let Some(plugin_name) = declared else {
        return Ok(SessionWrap::NoWrap);
    };

    let scope = wrap_scope(&args.dot_flox_path, plugin_name);
    match std::env::var(SESSION_WRAPPED_VAR) {
        Ok(marker) if marker == scope => {
            debug!(
                plugin = plugin_name,
                "session already wrapped; skipping session-wrap dispatch"
            );
            return Ok(SessionWrap::NoWrap);
        },
        Ok(_) => {
            bail!(formatdoc! {"
                Cannot activate this environment inside another environment's session-wrap boundary.
                Exit the wrapped session first, then run 'flox activate' again."});
        },
        Err(_) => {},
    }

    if args.is_ephemeral {
        debug!(
            plugin = plugin_name,
            "ephemeral activation skips session-wrap dispatch"
        );
        return Ok(SessionWrap::NoWrap);
    }

    if args.invocation_type.is_in_place() {
        bail!(formatdoc! {"
            Cannot activate in-place an environment that declares a session-wrap plugin.
            An 'eval \"$(flox activate)\"' cannot hand the current shell to plugin '{plugin_name}'.
            Run 'flox activate' to enter a wrapped session instead."});
    }

    let hook_path = hook_dir.join(plugin_name);
    if !hook_path.is_file() {
        bail!(formatdoc! {"
            Plugin '{plugin_name}' declares a session-wrap hook but the environment provides none.
            Expected an executable at {path}.
            Ensure the plugin package is installed and provides the hook, or remove the [plugin-hooks] declaration.",
        path = hook_path.display()});
    }

    let package = args
        .lockfile
        .packages
        .iter()
        .filter(|package| package.system() == args.system)
        .find(|package| package.install_id() == plugin_name);
    let Some(package) = package else {
        bail!(formatdoc! {"
            Plugin '{plugin_name}' is declared in [plugin-hooks] but not installed in this environment.
            Add a package with install id '{plugin_name}' to [install] before declaring its hooks."});
    };

    let canonical_hook = hook_path.canonicalize().with_context(|| {
        format!(
            "could not resolve the session-wrap hook at {}",
            hook_path.display()
        )
    })?;
    let store_paths = package_store_paths(package);
    if !store_paths
        .iter()
        .any(|store_path| canonical_hook.starts_with(store_path))
    {
        bail!(formatdoc! {"
            The session-wrap hook for plugin '{plugin_name}' is provided by a different package.
            Hooks must be shipped by the declared plugin's own package.
            Remove the conflicting package or fix the [plugin-hooks] declaration."});
    }

    let metadata = canonical_hook.metadata().with_context(|| {
        format!(
            "could not read the session-wrap hook at {}",
            canonical_hook.display()
        )
    })?;
    if metadata.permissions().mode() & 0o111 == 0 {
        bail!(formatdoc! {"
            The session-wrap hook for plugin '{plugin_name}' is not executable.
            The plugin package must ship {SESSION_WRAP_HOOK_DIR}/{plugin_name} with the executable bit set."});
    }

    let plugin_table = args
        .manifest
        .plugins
        .inner()
        .get(plugin_name)
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    let inner_argv = std::iter::once(
        std::env::current_exe()
            .ok()
            .and_then(|p| p.to_str().map(String::from))
            .unwrap_or_else(|| "flox".to_string()),
    )
    .chain(std::env::args().skip(1))
    .collect();

    let ctx = SessionWrapCtx {
        ctx_version: 1,
        dot_flox_path: args.dot_flox_path,
        env_name: args.env_name,
        activation_mode: args.activation_mode,
        rendered_env: args.rendered_env,
        lockfile_path: args.lockfile_path,
        plugin_table,
        invocation_type: args.invocation_type.clone(),
        stdin_is_tty: std::io::stdin().is_terminal(),
        stdout_is_tty: std::io::stdout().is_terminal(),
        inner_argv,
        wrap_scope: scope,
    };

    Ok(SessionWrap::Wrap(Box::new(SessionWrapExec {
        hook_path: canonical_hook,
        plugin_name: plugin_name.to_string(),
        ctx,
    })))
}

impl SessionWrapExec {
    /// Exec the hook, which re-enters the activation under its boundary and
    /// never returns on success. `Infallible` makes that contract visible in
    /// the type; an `Err` means the launch itself failed.
    ///
    /// The ctx file is `0600` in flox's temp dir. The hook execs away, so
    /// nothing can delete it deterministically; it is cleaned up with the
    /// temp dir rather than by its consumer.
    pub fn exec(self, temp_dir: &Path) -> Result<Infallible> {
        let tempfile = tempfile::NamedTempFile::new_in(temp_dir)
            .context("could not create the session-wrap ctx file")?;
        let writer = BufWriter::new(&tempfile);
        serde_json::to_writer_pretty(writer, &self.ctx)
            .context("could not serialize the session-wrap ctx")?;
        let (_, ctx_path) = tempfile
            .keep()
            .context("could not persist the session-wrap ctx file")?;

        let flox_bin = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("flox"));

        debug!(
            plugin = self.plugin_name,
            hook = %self.hook_path.display(),
            "exec'ing session-wrap hook"
        );
        let mut command = std::process::Command::new(&self.hook_path);
        command
            .env(FLOX_HOOK_CTX_VAR, &ctx_path)
            .env(FLOX_HOOK_VAR, "session-wrap")
            .env(FLOX_PLUGIN_NAME_VAR, &self.plugin_name)
            .env(FLOX_BIN_VAR, &flox_bin)
            .env(FLOX_HOOK_JQ_VAR, &*JQ_BIN);

        // exec never returns on success
        let err = command.exec();
        Err(err).with_context(|| {
            format!(
                "failed to execute the session-wrap hook for plugin '{}' at {}",
                self.plugin_name,
                self.hook_path.display()
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_scope_is_stable_and_distinguishes_plugin_and_path() {
        let scope = wrap_scope(Path::new("/proj/.flox"), "openshell");
        assert_eq!(scope, wrap_scope(Path::new("/proj/.flox"), "openshell"));
        assert_ne!(scope, wrap_scope(Path::new("/proj/.flox"), "other"));
        assert_ne!(scope, wrap_scope(Path::new("/other/.flox"), "openshell"));
    }
}
