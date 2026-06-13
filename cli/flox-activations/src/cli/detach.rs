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

        let empty_start_id = state.detach(self.pid)?;

        // In the executive we only remove start state dir when we know
        // cleanup_all won't trigger, but we can't know whether or not
        // cleanup_all will trigger here because we're about to drop the lock.
        // It won't hurt to remove this directory
        if let Some(ref start_id) = empty_start_id {
            start_id
                .remove_start_state_dir(&self.activation_state_dir)
                .context("failed to remove start state dir after detach")?;
        }

        // This should trigger the executive to check if it needs to cleanup
        write_activations_json(&state, &activations_json_path, lock)
            .context("failed to write state.json after detach")?;

        Ok(())
    }
}

#[cfg(test)]
mod test {
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
        let start_id = StartIdentifier::new("/nix/store/test");
        state.start_or_attach(pid, &start_id.store_path);
        write_activation_state(tmp.path(), &dot_flox_path, state);
        let activation_state_dir = activation_state_dir_path(tmp.path(), &dot_flox_path);
        let start_state_dir = start_id.start_state_dir(&activation_state_dir).unwrap();
        std::fs::create_dir_all(&start_state_dir).unwrap();

        let activation_state_dir = activation_state_dir_path(tmp.path(), &dot_flox_path);

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

    /// When the last PID overall detaches, its start state dir is removed
    /// immediately rather than deferred to the executive's cleanup_all — so a
    /// racing activation that supersedes this start can't leave it orphaned.
    #[test]
    fn start_state_dir_removed_when_last_pid_detaches() {
        let tmp = TempDir::new().unwrap();
        let dot_flox_path = tmp.path().join(".flox");
        let pid = 12345_i32;

        let mut state = ActivationState::new(
            &ActivateMode::default(),
            Some(&dot_flox_path),
            dot_flox_path.join("run/default"),
        );
        state.set_executive_pid(1);
        let StartOrAttachResult::Start { start_id } = state.start_or_attach(pid, "/nix/store/test")
        else {
            panic!("expected Start for pid");
        };

        write_activation_state(tmp.path(), &dot_flox_path, state);

        let activation_state_dir = activation_state_dir_path(tmp.path(), &dot_flox_path);

        let start_state_dir = start_id.start_state_dir(&activation_state_dir).unwrap();
        std::fs::create_dir_all(&start_state_dir).unwrap();
        assert!(
            start_state_dir.exists(),
            "start state dir should exist before detach"
        );

        let args = DetachArgs {
            activation_state_dir,
            pid,
        };
        args.handle().expect("detach should succeed");

        assert!(
            !start_state_dir.exists(),
            "start state dir should be removed when the last PID detaches"
        );
    }
}
