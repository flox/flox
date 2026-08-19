use std::path::PathBuf;

use anyhow::Context;
use clap::Args;
use flox_core::activations::{read_activations_json, state_json_path, write_activations_json};
use tracing::{debug, warn};

use crate::Error;

/// Detach a PID from an activation, updating state.json accordingly.
///
/// This is the deferred equivalent of the inline detach that was previously
/// performed inside `flox deactivate --print-script`. By emitting a
/// `flox-activations detach` command in the deactivation script and having
/// the shell eval it, we keep the binary side-effect-free during
/// `--print-script` and avoid needing a state.json schema version bump.
#[derive(Debug, Args)]
pub struct DetachArgs {
    #[arg(help = "The base directory for activation state.")]
    #[arg(long, value_name = "PATH")]
    pub activation_state_dir: PathBuf,
    #[arg(help = "The PID of the shell detaching from the activation.")]
    #[arg(short, long, value_name = "PID")]
    pub pid: i32,
}

impl DetachArgs {
    pub fn handle(self) -> Result<(), Error> {
        let activations_json_path = state_json_path(&self.activation_state_dir);

        let (activation_state_opt, lock) = read_activations_json(&activations_json_path)
            .with_context(|| {
                format!(
                    "failed to read state.json at '{}'",
                    activations_json_path.display()
                )
            })?;

        let Some(mut state) = activation_state_opt else {
            // The activation's executive removes the whole state directory as
            // soon as the last PID detaches. The prompt hook emits this `detach`
            // unconditionally and races that async cleanup, so a missing state
            // file means the work is already done — the PID is no longer
            // attached. Treat it as a no-op rather than surfacing a spurious
            // error on the user's prompt.
            debug!(
                pid = self.pid,
                path = %activations_json_path.display(),
                "no activation state to detach from; assuming already cleaned up"
            );
            return Ok(());
        };

        state.detach(self.pid)?;

        // An emptied start is normally torn down by the executive, woken by
        // the state.json write below: it removes the start from state.json,
        // runs its hook.on-deactivate, and removes the start state dir.
        // Without a running executive (e.g. containerize uses its own PID)
        // nothing would ever do that, so tear orphaned starts down inline; no
        // hook runs in that case.
        let orphaned = if state.executive_running() {
            Vec::new()
        } else {
            state.remove_orphaned_starts().orphaned
        };

        // This should trigger the executive to check if it needs to cleanup
        write_activations_json(&state, &activations_json_path, lock)
            .context("failed to write state.json after detach")?;

        // Remove the orphaned starts' dirs only after their removal has been
        // persisted, mirroring the executive's mutate -> write -> remove
        // ordering. Removal is best-effort, like the executive's sweep.
        for start_id in orphaned {
            if let Err(err) = start_id.remove_start_state_dir(&self.activation_state_dir) {
                warn!(%err, ?start_id, "failed to remove start state dir after detach");
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use flox_core::activate::mode::ActivateMode;
    use flox_core::activations::test_helpers::{read_activation_state, write_activation_state};
    use flox_core::activations::{ActivationState, StartOrAttachResult, activation_state_dir_path};
    use tempfile::TempDir;

    use super::DetachArgs;

    /// Successful detach removes the PID from state.json.
    #[test]
    fn successful_detach_removes_pid() {
        let tmp = TempDir::new().unwrap();
        let dot_flox_path = tmp.path().join(".flox");
        let pid = 12345_i32;

        let mut state = ActivationState::new(
            &ActivateMode::default(),
            Some(dot_flox_path.clone()),
            dot_flox_path.join("run/default"),
        );
        state.set_executive_pid(1);
        // Use the identifier minted by start_or_attach: StartIdentifier is
        // timestamped, so a locally constructed one names a different start
        // state dir whenever the millisecond ticks in between.
        let StartOrAttachResult::Start { start_id } = state.start_or_attach(pid, "/nix/store/test")
        else {
            panic!("expected Start for pid");
        };
        write_activation_state(tmp.path(), &dot_flox_path, state);
        let activation_state_dir = activation_state_dir_path(tmp.path(), &dot_flox_path);
        let start_state_dir = start_id.start_state_dir(&activation_state_dir).unwrap();
        std::fs::create_dir_all(&start_state_dir).unwrap();

        let args = DetachArgs {
            activation_state_dir,
            pid,
        };
        args.handle().expect("detach should succeed");

        let updated = read_activation_state(tmp.path(), &dot_flox_path);
        assert!(
            updated.attached_pids_is_empty(),
            "PID should be removed from state.json after detach"
        );
    }

    /// Detaching when the state file is already gone (the executive cleaned it
    /// up first) is a no-op success, not an error — the prompt hook races that
    /// async cleanup.
    #[test]
    fn detach_with_missing_state_is_ok() {
        let tmp = TempDir::new().unwrap();
        let dot_flox_path = tmp.path().join(".flox");
        let activation_state_dir = activation_state_dir_path(tmp.path(), &dot_flox_path);
        // No state.json is ever written.

        let args = DetachArgs {
            activation_state_dir,
            pid: 12345,
        };
        args.handle()
            .expect("detach should be a no-op when state is missing");
    }

    /// Set up state with a single attached PID and its start state dir,
    /// returning the activation state dir and the start state dir.
    fn setup_single_attachment(
        tmp: &TempDir,
        dot_flox_path: &std::path::Path,
        pid: i32,
        executive_pid: i32,
    ) -> (std::path::PathBuf, std::path::PathBuf) {
        let mut state = ActivationState::new(
            &ActivateMode::default(),
            Some(dot_flox_path),
            dot_flox_path.join("run/default"),
        );
        state.set_executive_pid(executive_pid);
        let StartOrAttachResult::Start { start_id } = state.start_or_attach(pid, "/nix/store/test")
        else {
            panic!("expected Start for pid");
        };
        // Mirror the real lifecycle: the activation completes its hooks and
        // marks the start ready before any detach happens. A start still in
        // `Starting` is never torn down by detach.
        state.set_ready(&start_id);

        write_activation_state(tmp.path(), dot_flox_path, state);

        let activation_state_dir = activation_state_dir_path(tmp.path(), dot_flox_path);
        let start_state_dir = start_id.start_state_dir(&activation_state_dir).unwrap();
        std::fs::create_dir_all(&start_state_dir).unwrap();
        assert!(
            start_state_dir.exists(),
            "start state dir should exist before detach"
        );
        (activation_state_dir, start_state_dir)
    }

    /// When the last PID detaches and an executive is running, the start
    /// state dir is left in place: the executive runs hook.on-deactivate and
    /// removes it.
    #[test]
    fn start_state_dir_handed_to_executive_when_last_pid_detaches() {
        let tmp = TempDir::new().unwrap();
        let dot_flox_path = tmp.path().join(".flox");
        let pid = 12345_i32;

        // PID 1 is always running, standing in for a live executive.
        let (activation_state_dir, start_state_dir) =
            setup_single_attachment(&tmp, &dot_flox_path, pid, 1);

        let args = DetachArgs {
            activation_state_dir,
            pid,
        };
        args.handle().expect("detach should succeed");

        assert!(
            start_state_dir.exists(),
            "start state dir should be left for the executive when it is running"
        );
    }

    /// Without a running executive (e.g. containerize) nothing would ever
    /// sweep the start state dir, so detach removes it inline.
    #[test]
    fn start_state_dir_removed_inline_without_executive() {
        let tmp = TempDir::new().unwrap();
        let dot_flox_path = tmp.path().join(".flox");
        let pid = 12345_i32;

        // An executive PID that is certainly not running.
        let dead_executive = start_dead_pid();
        let (activation_state_dir, start_state_dir) =
            setup_single_attachment(&tmp, &dot_flox_path, pid, dead_executive);

        let args = DetachArgs {
            activation_state_dir,
            pid,
        };
        args.handle().expect("detach should succeed");

        assert!(
            !start_state_dir.exists(),
            "start state dir should be removed inline when no executive is running"
        );
    }

    /// With two starts and no executive, detaching the last PID of one start
    /// removes only that start's dir; the still-attached start survives.
    #[test]
    fn inline_teardown_only_removes_emptied_start() {
        let tmp = TempDir::new().unwrap();
        let dot_flox_path = tmp.path().join(".flox");

        let mut state = ActivationState::new(
            &ActivateMode::default(),
            Some(&dot_flox_path),
            dot_flox_path.join("run/default"),
        );
        state.set_executive_pid(start_dead_pid());
        let StartOrAttachResult::Start { start_id: start_1 } =
            state.start_or_attach(111, "/nix/store/one")
        else {
            panic!("expected Start");
        };
        state.set_ready(&start_1);
        // A second activation supersedes the first with a new store path.
        let StartOrAttachResult::Start { start_id: start_2 } =
            state.start_or_attach(222, "/nix/store/two")
        else {
            panic!("expected Start");
        };
        state.set_ready(&start_2);

        write_activation_state(tmp.path(), &dot_flox_path, state);
        let activation_state_dir = activation_state_dir_path(tmp.path(), &dot_flox_path);
        let dir_1 = start_1.start_state_dir(&activation_state_dir).unwrap();
        let dir_2 = start_2.start_state_dir(&activation_state_dir).unwrap();
        std::fs::create_dir_all(&dir_1).unwrap();
        std::fs::create_dir_all(&dir_2).unwrap();

        let args = DetachArgs {
            activation_state_dir,
            pid: 111,
        };
        args.handle().expect("detach should succeed");

        assert!(!dir_1.exists(), "the emptied start's dir should be removed");
        assert!(
            dir_2.exists(),
            "the still-attached start's dir should survive"
        );
    }

    /// A start state dir that is already gone (e.g. removed by an earlier
    /// crashed detach) must not wedge the detach: the PID removal still has
    /// to be persisted to state.json.
    #[test]
    fn detach_persists_even_when_start_state_dir_is_missing() {
        let tmp = TempDir::new().unwrap();
        let dot_flox_path = tmp.path().join(".flox");
        let pid = 12345_i32;

        let dead_executive = start_dead_pid();
        let (activation_state_dir, start_state_dir) =
            setup_single_attachment(&tmp, &dot_flox_path, pid, dead_executive);
        std::fs::remove_dir_all(&start_state_dir).unwrap();

        let args = DetachArgs {
            activation_state_dir,
            pid,
        };
        args.handle()
            .expect("detach should succeed despite the missing start state dir");

        let updated = read_activation_state(tmp.path(), &dot_flox_path);
        assert!(
            updated.attached_pids_is_empty(),
            "the detach should be persisted to state.json"
        );
    }

    /// Spawn and reap a short-lived process, returning its no-longer-running PID.
    fn start_dead_pid() -> i32 {
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id() as i32;
        child.wait().unwrap();
        pid
    }
}
