use std::path::PathBuf;

use anyhow::{Context, anyhow};
use clap::Args;
use flox_core::activations::{read_activations_json, state_json_path, write_activations_json};

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
            return Err(anyhow!(
                "No activation state found at '{}'; cannot detach PID {}",
                activations_json_path.display(),
                self.pid
            ));
        };

        let empty_start_id = state.detach(self.pid);

        // Per ActivationState::detach doc contract: update_ready_after_detach
        // must be called after detach, but only when there are still
        // attached PIDs (the function panics on an empty map).
        if !state.attached_pids_is_empty() {
            state.update_ready_after_detach();
            // Remove the start-state dir only when other PIDs remain.
            // When PIDs are still present, the executive cannot call
            // cleanup_all (which renames the parent dir), so there is no
            // race window here.  When this is the last PID overall,
            // the executive cleans up everything via StateFileChanged →
            // cleanup_all after we write below.
            if let Some(ref start_id) = empty_start_id {
                start_id
                    .remove_start_state_dir(&self.activation_state_dir)
                    .context("failed to remove start state dir after detach")?;
            }
        }

        // Always write: state.detach() has already mutated in-memory state.
        // The executive is always running at detach time.  When 0 PIDs remain
        // it will observe the change via StateFileChanged and call cleanup_all.
        write_activations_json(&state, &activations_json_path, lock)
            .context("failed to write state.json after detach")?;

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use std::path::PathBuf;

    use flox_core::activate::context::InvocationType;
    use flox_core::activate::mode::ActivateMode;
    use flox_core::activations::test_helpers::{read_activation_state, write_activation_state};
    use flox_core::activations::{
        ActivationState,
        StartIdentifier,
        StartOrAttachResult,
        activation_state_dir_path,
    };
    use tempfile::TempDir;

    use super::DetachArgs;

    fn make_state_with_pid(
        dot_flox_path: &std::path::Path,
        pid: i32,
        invocation_type: InvocationType,
    ) -> ActivationState {
        let mut state = ActivationState::new(
            &ActivateMode::default(),
            Some(dot_flox_path),
            dot_flox_path.join("run/default"),
        );
        state.set_executive_pid(1);
        let start_id = StartIdentifier::new("/nix/store/test");
        state.start_or_attach(pid, &start_id.store_path, invocation_type);
        state
    }

    /// Successful detach removes the PID from state.json.
    #[test]
    fn successful_detach_removes_pid() {
        let tmp = TempDir::new().unwrap();
        let dot_flox_path = tmp.path().join(".flox");
        let pid = 12345_i32;

        let state = make_state_with_pid(&dot_flox_path, pid, InvocationType::InPlace);
        write_activation_state(tmp.path(), &dot_flox_path, state);

        let activation_state_dir = activation_state_dir_path(tmp.path(), &dot_flox_path);

        let args = DetachArgs {
            activation_state_dir,
            pid,
        };
        args.handle().expect("detach should succeed");

        let updated = read_activation_state(tmp.path(), &dot_flox_path);
        assert!(
            !updated.is_pid_attached(pid),
            "PID should be removed from state.json after detach"
        );
    }

    /// When there is no state.json, detach returns an error.
    #[test]
    fn no_state_json_returns_error() {
        let tmp = TempDir::new().unwrap();
        let dot_flox_path = PathBuf::from("/nonexistent/.flox");
        let activation_state_dir = activation_state_dir_path(tmp.path(), &dot_flox_path);

        // Create the dir so lock acquisition succeeds, but don't write state.json.
        std::fs::create_dir_all(&activation_state_dir).unwrap();

        let args = DetachArgs {
            activation_state_dir,
            pid: 42,
        };
        let result = args.handle();
        assert!(
            result.is_err(),
            "detach should fail when state.json is absent"
        );
    }

    /// Detach always writes state.json, even when the last PID is removed.
    /// The executive observes the change via StateFileChanged and calls cleanup_all.
    #[test]
    fn last_pid_detach_writes_state() {
        let tmp = TempDir::new().unwrap();
        let dot_flox_path = tmp.path().join(".flox");
        let pid = 12345_i32;

        let state = make_state_with_pid(&dot_flox_path, pid, InvocationType::InPlace);
        write_activation_state(tmp.path(), &dot_flox_path, state);

        let activation_state_dir = activation_state_dir_path(tmp.path(), &dot_flox_path);

        let args = DetachArgs {
            activation_state_dir,
            pid,
        };
        args.handle().expect("detach should succeed");

        let updated = read_activation_state(tmp.path(), &dot_flox_path);
        assert!(
            !updated.is_pid_attached(pid),
            "PID should be absent from state.json after detach"
        );
    }

    /// When the last PID for a start detaches but other PIDs remain (attached
    /// to different starts), the empty start's state directory is removed.
    /// When this is the last PID overall, the executive handles dir cleanup via
    /// StateFileChanged → cleanup_all.
    #[test]
    fn start_state_dir_removed_when_other_pids_remain() {
        let tmp = TempDir::new().unwrap();
        let dot_flox_path = tmp.path().join(".flox");
        let pid1 = 12345_i32; // will be detached — last PID for start_id1
        let pid2 = 99999_i32; // stays attached to a different start

        // Build a state with pid1 on start_id1 and pid2 on start_id2 so that
        // after detaching pid1, attached_pids is not empty (pid2 remains).
        let mut state = ActivationState::new(
            &ActivateMode::default(),
            Some(&dot_flox_path),
            dot_flox_path.join("run/default"),
        );
        state.set_executive_pid(1);

        // Attach pid1 → starts start_id1, ready becomes Starting(pid1, start_id1)
        let r1 = state.start_or_attach(pid1, "/nix/store/test1", InvocationType::InPlace);
        let StartOrAttachResult::Start {
            start_id: start_id1,
        } = r1
        else {
            panic!("expected Start for pid1");
        };
        // Mark ready so pid2 can trigger a fresh Start on a different store path
        state.set_ready(&start_id1);

        // Attach pid2 with a different store path → starts start_id2
        state.start_or_attach(pid2, "/nix/store/test2", InvocationType::InPlace);

        write_activation_state(tmp.path(), &dot_flox_path, state);

        let activation_state_dir = activation_state_dir_path(tmp.path(), &dot_flox_path);

        // Create start_id1's directory on disk (the one that should be removed).
        let start_state_dir1 = start_id1.start_state_dir(&activation_state_dir).unwrap();
        std::fs::create_dir_all(&start_state_dir1).unwrap();
        assert!(
            start_state_dir1.exists(),
            "start state dir should exist before detach"
        );

        let args = DetachArgs {
            activation_state_dir,
            pid: pid1,
        };
        args.handle().expect("detach should succeed");

        assert!(
            !start_state_dir1.exists(),
            "start state dir for detached start should be removed when other PIDs remain"
        );
    }
}
