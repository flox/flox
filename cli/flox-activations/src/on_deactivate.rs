//! Runs `hook.on-deactivate` when the last attachment detaches from a start.
//!
//! `hook.on-activate` runs once per start; this module provides the matching
//! bookend, executed by the executive as part of tearing down a start state
//! directory. The hook runs in a flox provided bash with the environment as
//! `hook.on-activate` left it (replayed from the start's env trace), with
//! output captured into the executive log. Failures are logged and never
//! block cleanup, and callers must not hold the state.json lock while the
//! hook runs so a hanging script can't block new activations.

use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};
use flox_core::activate::context::{AttachCtx, AttachProjectCtx};
use flox_core::activations::StartIdentifier;
use tracing::{debug, info, warn};

use crate::attach_diff::AttachDiff;
use crate::env_trace::EnvTrace;
use crate::vars_from_env::VarsFromEnvironment;

const BASH_BIN: &str = env!("X_BASH_BIN");

/// Relative path of the rendered `hook.on-deactivate` script within an
/// environment's store path.
const HOOK_ON_DEACTIVATE: &str = "activate.d/hook-on-deactivate";

/// Relative path of the plugin teardown hook directory within an
/// environment's store path. Scripts here are shipped by installed plugin
/// packages and sourced in lexical order before the user's
/// `hook.on-deactivate`, with `flox_plugin_data` available via the
/// interpreter's helpers.
const PLUGIN_ON_DEACTIVATE_DIR: &str = "etc/flox/hooks/on-deactivate.d";

/// Relative path of the activation helpers within the interpreter, sourced
/// before plugin teardown scripts so they can call `flox_plugin_data`.
const INTERPRETER_HELPERS: &str = "activate.d/helpers.bash";

/// Tear down the given start state directories, running each start's
/// `hook.on-deactivate` first.
///
/// The `orphaned` list must come from
/// `ActivationState::remove_orphaned_starts`, called under the state.json
/// lock and persisted so a start is only ever handed to this sweep once. The
/// lock must be dropped before calling this: the hook has no timeout and must
/// not block new activations.
pub fn sweep_orphaned_starts(
    subsystem_verbosity: u32,
    attach_ctx: &AttachCtx,
    project: &AttachProjectCtx,
    activation_state_dir: &Path,
    orphaned: Vec<StartIdentifier>,
) {
    for start_id in orphaned {
        info!(
            ?start_id,
            "tearing down start with no remaining attachments"
        );
        run_on_deactivate_hook(
            subsystem_verbosity,
            attach_ctx,
            project,
            &start_id,
            activation_state_dir,
        );
        if let Err(err) = start_id.remove_start_state_dir(activation_state_dir) {
            warn!(%err, ?start_id, "failed to remove start state dir");
        }
    }
}

/// Run the `hook.on-deactivate` script for a single start, if the rendered
/// environment has one.
///
/// Failures (including a failing script) are logged and swallowed so that
/// cleanup always proceeds.
pub fn run_on_deactivate_hook(
    subsystem_verbosity: u32,
    attach_ctx: &AttachCtx,
    project: &AttachProjectCtx,
    start_id: &StartIdentifier,
    activation_state_dir: &Path,
) {
    run_plugin_on_deactivate_scripts(
        subsystem_verbosity,
        attach_ctx,
        project,
        start_id,
        activation_state_dir,
    );

    let script = start_id.store_path.join(HOOK_ON_DEACTIVATE);
    if !script.exists() {
        debug!(?script, "no hook.on-deactivate script, skipping");
        return;
    }
    if let Err(err) = run_hook_script(
        subsystem_verbosity,
        attach_ctx,
        project,
        start_id,
        activation_state_dir,
        &script,
    ) {
        warn!(%err, ?script, "failed to run hook.on-deactivate; continuing cleanup");
    }
}

/// Source the plugin teardown scripts shipped in the rendered environment's
/// `etc/flox/hooks/on-deactivate.d`, in lexical order, in a single bash with
/// the interpreter's helpers preloaded (functions don't replay from the env
/// trace, so `flox_plugin_data` needs explicit wiring). Runs before the
/// user's `hook.on-deactivate` and shares its posture: replayed activation
/// environment, output to the executive log, failures swallowed.
///
/// Gated on the activation's recorded `plugin_hooks` flag; there is nothing
/// to do for activations that never armed plugin hooks.
fn run_plugin_on_deactivate_scripts(
    subsystem_verbosity: u32,
    attach_ctx: &AttachCtx,
    project: &AttachProjectCtx,
    start_id: &StartIdentifier,
    activation_state_dir: &Path,
) {
    if !attach_ctx.plugin_hooks {
        return;
    }
    let hook_dir = start_id.store_path.join(PLUGIN_ON_DEACTIVATE_DIR);
    let Ok(entries) = std::fs::read_dir(&hook_dir) else {
        debug!(?hook_dir, "no plugin on-deactivate.d directory, skipping");
        return;
    };
    let mut scripts: Vec<_> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "sh"))
        .collect();
    if scripts.is_empty() {
        return;
    }
    scripts.sort();

    if let Err(err) = run_plugin_scripts(
        subsystem_verbosity,
        attach_ctx,
        project,
        start_id,
        activation_state_dir,
        &scripts,
    ) {
        warn!(%err, ?hook_dir, "failed to run plugin on-deactivate scripts; continuing cleanup");
    }
}

fn run_plugin_scripts(
    subsystem_verbosity: u32,
    attach_ctx: &AttachCtx,
    project: &AttachProjectCtx,
    start_id: &StartIdentifier,
    activation_state_dir: &Path,
    scripts: &[std::path::PathBuf],
) -> Result<()> {
    let start_state_dir = start_id.start_state_dir(activation_state_dir)?;

    let vars_from_env = VarsFromEnvironment::get()?;
    let env_trace = EnvTrace::from_state_dir(&start_state_dir)
        .context("start has no usable env trace; hook.on-activate never completed")?;
    let attach_diff = AttachDiff::new(
        attach_ctx,
        Some(project),
        subsystem_verbosity,
        vars_from_env,
        &env_trace,
        false,
    )?;

    let helpers = attach_ctx.interpreter_path.join(INTERPRETER_HELPERS);

    let mut command = Command::new(BASH_BIN);
    attach_diff.apply_to_command(&mut command);
    // Paths are passed as arguments rather than interpolated into the script
    // text so they need no quoting.
    command
        .arg("-c")
        .arg(r#"helpers="$1"; shift; if [ -e "$helpers" ]; then source "$helpers"; fi; for script in "$@"; do source "$script"; done"#)
        .arg("plugin-on-deactivate")
        .arg(&helpers)
        .args(scripts);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    info!(?scripts, "running plugin on-deactivate scripts");
    let output = command
        .output()
        .context("failed to spawn plugin on-deactivate scripts")?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stdout.is_empty() {
        info!(%stdout, "plugin on-deactivate stdout");
    }
    if !stderr.is_empty() {
        info!(%stderr, "plugin on-deactivate stderr");
    }
    if !output.status.success() {
        warn!(
            status = %output.status,
            "plugin on-deactivate scripts failed; continuing cleanup"
        );
    }
    Ok(())
}

fn run_hook_script(
    subsystem_verbosity: u32,
    attach_ctx: &AttachCtx,
    project: &AttachProjectCtx,
    start_id: &StartIdentifier,
    activation_state_dir: &Path,
    script: &Path,
) -> Result<()> {
    let start_state_dir = start_id.start_state_dir(activation_state_dir)?;

    // Replay the activation's environment so the hook sees variables
    // exported by hook.on-activate. A start without a usable trace never got
    // through activation, so its bookend never opened; skip.
    let vars_from_env = VarsFromEnvironment::get()?;
    let env_trace = EnvTrace::from_state_dir(&start_state_dir)
        .context("start has no usable env trace; hook.on-activate never completed")?;
    let attach_diff = AttachDiff::new(
        attach_ctx,
        Some(project),
        subsystem_verbosity,
        vars_from_env,
        &env_trace,
        false,
    )?;

    let mut command = Command::new(BASH_BIN);
    attach_diff.apply_to_command(&mut command);
    command.arg(script);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    info!(?script, "running hook.on-deactivate");
    let output = command
        .output()
        .context("failed to spawn hook.on-deactivate")?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stdout.is_empty() {
        info!(%stdout, "hook.on-deactivate stdout");
    }
    if !stderr.is_empty() {
        info!(%stderr, "hook.on-deactivate stderr");
    }
    if !output.status.success() {
        warn!(
            status = %output.status,
            "hook.on-deactivate failed; continuing cleanup"
        );
    }
    Ok(())
}

#[cfg(test)]
mod test {
    use std::path::PathBuf;

    use flox_core::activations::StartIdentifier;
    use tempfile::TempDir;

    use super::*;
    use crate::env_trace::ENV_TRACE_LOG;

    /// Create minimal context for the hook runner.
    fn test_context(dot_flox_path: &Path) -> (AttachCtx, AttachProjectCtx) {
        let attach = AttachCtx {
            env: "test".to_string(),
            env_description: "test".to_string(),
            env_cache: dot_flox_path.join("cache"),
            interpreter_path: PathBuf::from("/nix/store/fake"),
            prompt_color_1: "".to_string(),
            prompt_color_2: "".to_string(),
            flox_prompt_environments: "".to_string(),
            set_prompt: false,
            flox_env_cuda_detection: "".to_string(),
            add_sbin: false,
            flox_active_environments: "".to_string(),
            plugin_hooks: true,
        };
        let project = AttachProjectCtx {
            env_project: dot_flox_path.to_path_buf(),
            dot_flox_path: dot_flox_path.to_path_buf(),
            flox_env_log_dir: PathBuf::from("/tmp/test_log_dir"),
            flox_services_socket: PathBuf::from("/does_not_exist"),
            process_compose_bin: PathBuf::from("/nix/store/fake-process-compose"),
            services_to_start: Vec::new(),
        };
        (attach, project)
    }

    /// Set up a fake rendered environment and a start for it, optionally with
    /// a hook.on-deactivate script. Returns the start id and its state dir.
    fn setup_start(tmp: &TempDir, script: Option<&str>) -> (StartIdentifier, PathBuf, PathBuf) {
        let store_path = tmp.path().join("store-path");
        if let Some(script) = script {
            let activate_d = store_path.join("activate.d");
            std::fs::create_dir_all(&activate_d).unwrap();
            std::fs::write(activate_d.join("hook-on-deactivate"), script).unwrap();
        }

        let activation_state_dir = tmp.path().join("activations");
        let start_id = StartIdentifier::new(&store_path);
        let start_state_dir = start_id.start_state_dir(&activation_state_dir).unwrap();
        std::fs::create_dir_all(&start_state_dir).unwrap();
        // Trace recorded during activation: hook.on-activate exported
        // ON_ACTIVATE_VAR. Fields are timestamp, op, exported flags, name,
        // old value (@ = none), and operand, separated by US (0x1f).
        std::fs::write(
            start_state_dir.join(ENV_TRACE_LOG),
            "1\u{1f}set\u{1f}11\u{1f}ON_ACTIVATE_VAR\u{1f}@\u{1f}:from-on-activate\n",
        )
        .unwrap();

        (start_id, activation_state_dir, start_state_dir)
    }

    /// The sweep runs the hook with the end-of-activation environment
    /// replayed, then removes the start state dir.
    #[test]
    fn sweep_runs_hook_with_replayed_env_and_removes_dir() {
        let tmp = TempDir::new().unwrap();
        let marker = tmp.path().join("marker");
        let script = format!("echo -n \"$ON_ACTIVATE_VAR\" > '{}'\n", marker.display());
        let (start_id, activation_state_dir, start_state_dir) = setup_start(&tmp, Some(&script));

        let (attach, project) = test_context(tmp.path());
        sweep_orphaned_starts(0, &attach, &project, &activation_state_dir, vec![start_id]);

        assert_eq!(
            std::fs::read_to_string(&marker).expect("hook should have run"),
            "from-on-activate",
            "hook should see variables exported by hook.on-activate"
        );
        assert!(
            !start_state_dir.exists(),
            "start state dir should be removed after the hook ran"
        );
    }

    /// Without a script the sweep still removes the start state dir.
    #[test]
    fn sweep_removes_dir_when_no_script() {
        let tmp = TempDir::new().unwrap();
        let (start_id, activation_state_dir, start_state_dir) = setup_start(&tmp, None);

        let (attach, project) = test_context(tmp.path());
        sweep_orphaned_starts(0, &attach, &project, &activation_state_dir, vec![start_id]);

        assert!(!start_state_dir.exists());
    }

    /// Write plugin teardown scripts into the fake rendered environment.
    fn add_plugin_scripts(tmp: &TempDir, scripts: &[(&str, &str)]) {
        let hook_dir = tmp
            .path()
            .join("store-path")
            .join("etc/flox/hooks/on-deactivate.d");
        std::fs::create_dir_all(&hook_dir).unwrap();
        for (name, contents) in scripts {
            std::fs::write(hook_dir.join(name), contents).unwrap();
        }
    }

    /// Plugin on-deactivate.d scripts run in lexical order, with the
    /// replayed activation environment, before the user's
    /// hook.on-deactivate.
    #[test]
    fn sweep_runs_plugin_teardown_scripts_in_order_before_user_hook() {
        let tmp = TempDir::new().unwrap();
        let marker = tmp.path().join("marker");
        let user_hook = format!("echo \"user:$ON_ACTIVATE_VAR\" >> '{}'\n", marker.display());
        let (start_id, activation_state_dir, _) = setup_start(&tmp, Some(&user_hook));
        add_plugin_scripts(&tmp, &[
            (
                "2000_second.sh",
                &format!("echo second >> '{}'\n", marker.display()),
            ),
            (
                "1000_first.sh",
                &format!(
                    "echo \"first:$ON_ACTIVATE_VAR\" >> '{}'\n",
                    marker.display()
                ),
            ),
        ]);

        let (attach, project) = test_context(tmp.path());
        sweep_orphaned_starts(0, &attach, &project, &activation_state_dir, vec![start_id]);

        assert_eq!(
            std::fs::read_to_string(&marker).expect("scripts should have run"),
            "first:from-on-activate\nsecond\nuser:from-on-activate\n",
            "plugin scripts run in lexical order, with the replayed env, before the user hook"
        );
    }

    /// Without the recorded plugin_hooks gate, plugin teardown scripts are
    /// skipped while the user hook still runs.
    #[test]
    fn sweep_skips_plugin_teardown_scripts_when_gate_is_off() {
        let tmp = TempDir::new().unwrap();
        let marker = tmp.path().join("marker");
        let user_hook = format!("echo user >> '{}'\n", marker.display());
        let (start_id, activation_state_dir, _) = setup_start(&tmp, Some(&user_hook));
        add_plugin_scripts(&tmp, &[(
            "1000_plugin.sh",
            &format!("echo plugin >> '{}'\n", marker.display()),
        )]);

        let (mut attach, project) = test_context(tmp.path());
        attach.plugin_hooks = false;
        sweep_orphaned_starts(0, &attach, &project, &activation_state_dir, vec![start_id]);

        assert_eq!(
            std::fs::read_to_string(&marker).expect("user hook should have run"),
            "user\n",
            "plugin scripts must not run when the activation never armed plugin hooks"
        );
    }

    /// A failing hook doesn't block removal of the start state dir.
    #[test]
    fn sweep_removes_dir_when_hook_fails() {
        let tmp = TempDir::new().unwrap();
        let (start_id, activation_state_dir, start_state_dir) = setup_start(&tmp, Some("exit 1\n"));

        let (attach, project) = test_context(tmp.path());
        sweep_orphaned_starts(0, &attach, &project, &activation_state_dir, vec![start_id]);

        assert!(
            !start_state_dir.exists(),
            "a failing hook must not block cleanup"
        );
    }
}
