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
use flox_core::activations::{ActivationState, StartIdentifier, read_start_ids_from_disk};
use tracing::{debug, info, warn};

use crate::attach_diff::AttachDiff;
use crate::env_trace::EnvTrace;
use crate::vars_from_env::VarsFromEnvironment;

const BASH_BIN: &str = env!("X_BASH_BIN");

/// Relative path of the rendered `hook.on-deactivate` script within an
/// environment's store path.
const HOOK_ON_DEACTIVATE: &str = "activate.d/hook-on-deactivate";

/// Enumerate the start state directories whose start has no remaining
/// attachments.
///
/// This MUST be called while holding the state.json lock: a start being
/// created writes its directory before registering itself in state.json
/// (under the same lock), so enumerating without the lock could catch a
/// directory whose start isn't visible in `state` yet. The returned starts
/// can then be torn down after the lock drops — a start with no attachments
/// can never gain new ones (only `ready` starts can be attached to, and an
/// emptied start is never `ready`).
pub fn orphaned_start_ids(
    activation_state_dir: &Path,
    state: &ActivationState,
) -> Vec<StartIdentifier> {
    let live = state.live_start_ids();
    read_start_ids_from_disk(activation_state_dir)
        .into_iter()
        .filter(|start_id| !live.contains(start_id))
        .collect()
}

/// Tear down the given start state directories, running each start's
/// `hook.on-deactivate` first.
///
/// The `orphaned` list must come from [orphaned_start_ids], but the lock must
/// be dropped before calling this: the hook has no timeout and must not block
/// new activations. The sweep is idempotent — a start is torn down exactly
/// once because its directory is removed immediately after its hook runs,
/// and everything here runs on the executive's single event loop thread.
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
        start_id
            .write_to_start_state_dir(&activation_state_dir)
            .unwrap();
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

    /// A start is orphaned once its last attachment detaches, and not before.
    #[test]
    fn orphaned_start_ids_excludes_live_starts() {
        let tmp = TempDir::new().unwrap();
        let activation_state_dir = tmp.path().join("activations");
        let store_path = tmp.path().join("store-path");
        let pid = std::process::id() as i32;

        let mut state = flox_core::activations::ActivationState::new(
            &flox_core::activate::mode::ActivateMode::default(),
            Some(tmp.path().join(".flox")),
            &store_path,
        );
        let flox_core::activations::StartOrAttachResult::Start { start_id } =
            state.start_or_attach(pid, &store_path)
        else {
            panic!("expected Start");
        };
        state.set_ready(&start_id);
        std::fs::create_dir_all(start_id.start_state_dir(&activation_state_dir).unwrap()).unwrap();
        start_id
            .write_to_start_state_dir(&activation_state_dir)
            .unwrap();

        assert_eq!(
            orphaned_start_ids(&activation_state_dir, &state),
            Vec::new(),
            "a start with an attachment is not orphaned"
        );

        state.detach(pid).unwrap();
        assert_eq!(
            orphaned_start_ids(&activation_state_dir, &state),
            vec![start_id],
            "a start becomes orphaned when its last attachment detaches"
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
