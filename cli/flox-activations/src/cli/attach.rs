use std::path::PathBuf;

use clap::Args;
use flox_core::activations::rewrite::{self, StartIdentifier, UnixTimestampMillis};
use flox_core::activations::{self};
use time::{Duration, OffsetDateTime};

use crate::Error;

#[derive(Debug, Args)]
pub struct AttachArgs {
    #[arg(help = "The PID of the shell registering interest in the activation.")]
    #[arg(short, long, value_name = "PID")]
    pub pid: i32,
    #[arg(help = "The path to the .flox directory for the environment.")]
    #[arg(long, value_name = "PATH")]
    pub dot_flox_path: PathBuf,
    #[command(flatten)]
    pub exclusive: AttachExclusiveArgs,
    /// The path to the runtime directory keeping activation data.
    #[arg(long, value_name = "PATH")]
    pub runtime_dir: PathBuf,
    #[arg(help = "Together with timestamp this identifies the activation to attach to.")]
    #[arg(long, value_name = "PATH")]
    pub store_path: PathBuf,
    #[arg(help = "Together with store_path this identifies the activation to attach to.")]
    #[arg(long, value_name = "TIMESTAMP")]
    pub timestamp: UnixTimestampMillis,
}

#[derive(Debug, Args)]
#[group(required = true, multiple = false)]
pub struct AttachExclusiveArgs {
    #[arg(
        help = "How long to wait between termination of this PID and cleaning up its interest. The pid argument must already be attached to the activation, so this simply adds an expiration."
    )]
    #[arg(short, long, value_name = "TIME_MS")]
    pub timeout_ms: Option<u32>,
    #[arg(help = "Remove the specified PID when attaching to this activation.")]
    #[arg(short, long, value_name = "PID")]
    pub remove_pid: Option<i32>,
}

impl AttachArgs {
    pub fn handle(self) -> Result<(), Error> {
        self.handle_inner(OffsetDateTime::now_utc())
    }

    pub fn handle_inner(self, now: OffsetDateTime) -> Result<(), Error> {
        let start_id = StartIdentifier {
            store_path: self.store_path,
            timestamp: self.timestamp,
        };
        let activations_json_path =
            activations::state_json_path(&self.runtime_dir, &self.dot_flox_path);

        let (activation_state, lock) = rewrite::read_activations_json(&activations_json_path)?;
        let Some(mut activation_state) = activation_state else {
            anyhow::bail!(
                "Expected an existing state file at {}",
                activations_json_path.display()
            );
        };

        match self.exclusive {
            AttachExclusiveArgs {
                timeout_ms: Some(timeout_ms),
                remove_pid: None,
            } => {
                let expiration = now + Duration::milliseconds(timeout_ms as i64);
                activation_state.replace_attachment(
                    start_id,
                    self.pid,
                    self.pid,
                    Some(expiration),
                )?;
            },
            AttachExclusiveArgs {
                timeout_ms: None,
                remove_pid: Some(remove_pid),
            } => {
                activation_state.replace_attachment(start_id, remove_pid, self.pid, None)?;
            },
            // This should be unreachable due to the group constraints when constructed by clap
            _ => {
                anyhow::bail!("Exactly one of --timeout-ms or --remove-pid must be specified");
            },
        }

        rewrite::write_activations_json(&activation_state, &activations_json_path, lock)?;

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    use flox_core::activate::mode::ActivateMode;
    use flox_core::activations::rewrite::{
        ActivationState,
        StartOrAttachResult,
        read_activations_json,
        write_activations_json,
    };
    use flox_core::activations::{acquire_activations_json_lock, state_json_path};
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;
    use time::OffsetDateTime;

    use super::{AttachArgs, AttachExclusiveArgs};

    /// Helper to write an ActivationState to disk
    ///
    /// Takes ownership of state so we don't accidentally use it after e.g. a
    /// watcher modifies state on disk
    pub fn write_activation_state(
        runtime_dir: &Path,
        dot_flox_path: &Path,
        state: ActivationState,
    ) {
        let state_json_path = state_json_path(runtime_dir, dot_flox_path);
        let lock = acquire_activations_json_lock(&state_json_path).expect("failed to acquire lock");
        write_activations_json(&state, &state_json_path, lock).expect("failed to write state");
    }
    /// Helper to read an ActivationState from disk
    pub fn read_activation_state(runtime_dir: &Path, dot_flox_path: &Path) -> ActivationState {
        let state_json_path = state_json_path(runtime_dir, dot_flox_path);
        let (state, _lock) = read_activations_json(&state_json_path).expect("failed to read state");
        state.unwrap()
    }

    /// Attaching with a timeout adds an expiration to the (already) attached PID
    #[test]
    fn add_timeout_to_pid() {
        let runtime_dir = TempDir::new().unwrap();
        let flox_env = PathBuf::from("/path/to/floxenv");
        let pid = 1234;
        let store_path = PathBuf::from("/nix/store/test");

        // Create an activation with a PID attached
        let mut state = ActivationState::new(&ActivateMode::default());
        let result = state.start_or_attach(pid, &store_path);
        let StartOrAttachResult::Start { start_id, .. } = result else {
            panic!("Expected Start")
        };
        state.set_ready(&start_id);
        write_activation_state(runtime_dir.path(), &flox_env, state);

        // Attach the same PID with a timeout (replaces itself with expiration)
        let args = AttachArgs {
            dot_flox_path: flox_env.clone(),
            pid,
            store_path: start_id.store_path.clone(),
            timestamp: start_id.timestamp.clone(),
            exclusive: AttachExclusiveArgs {
                timeout_ms: Some(1000),
                remove_pid: None,
            },
            runtime_dir: runtime_dir.path().to_path_buf(),
        };

        let now = OffsetDateTime::now_utc();
        args.handle_inner(now).unwrap();

        let state = read_activation_state(runtime_dir.path(), &flox_env);

        let expected_attachments = BTreeMap::from([(start_id.clone(), vec![(
            pid,
            Some(now + time::Duration::milliseconds(1000)),
        )])]);
        assert_eq!(state.attachments_by_start_id(), expected_attachments);
    }

    /// Attaching with remove_pid replaces the attachment for the old PID with the new one
    #[test]
    fn attach_with_replace() {
        let runtime_dir = TempDir::new().unwrap();
        let flox_env = PathBuf::from("/path/to/floxenv");
        let old_pid = 1234;
        let new_pid = 5678;
        let store_path = PathBuf::from("store_path");

        // Create an activation with the old PID attached
        let mut state = ActivationState::new(&ActivateMode::default());
        let result = state.start_or_attach(old_pid, &store_path);
        let StartOrAttachResult::Start { start_id, .. } = result else {
            panic!("Expected Start")
        };
        state.set_ready(&start_id);
        write_activation_state(runtime_dir.path(), &flox_env, state);

        // Replace old PID with new PID
        let args = AttachArgs {
            dot_flox_path: flox_env.clone(),
            pid: new_pid,
            store_path: start_id.store_path.clone(),
            timestamp: start_id.timestamp.clone(),
            exclusive: AttachExclusiveArgs {
                timeout_ms: None,
                remove_pid: Some(old_pid),
            },
            runtime_dir: runtime_dir.path().to_path_buf(),
        };

        args.handle().unwrap();

        let activation = read_activation_state(runtime_dir.path(), &flox_env);

        let expected_attachments = BTreeMap::from([(start_id.clone(), vec![(new_pid, None)])]);
        assert_eq!(activation.attachments_by_start_id(), expected_attachments);
    }
}
