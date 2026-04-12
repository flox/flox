use std::path::PathBuf;

use anyhow::Result;
use clap::Args;
use flox_core::activations::{read_activations_json, state_json_path, write_activations_json};
use tracing::debug;

#[derive(Debug, Args)]
pub struct AutoDetachArgs {
    /// Shell PID to detach from the activation
    #[arg(long)]
    pub pid: i32,

    /// Path to the activation state directory
    #[arg(long)]
    pub activation_state_dir: PathBuf,
}

impl AutoDetachArgs {
    pub fn handle(self) -> Result<(), anyhow::Error> {
        let activations_json_path = state_json_path(&self.activation_state_dir);

        let (activations_opt, lock) = read_activations_json(&activations_json_path)?;

        let Some(mut activations) = activations_opt else {
            debug!(
                pid = self.pid,
                "no activation state found, nothing to detach"
            );
            return Ok(());
        };

        debug!(pid = self.pid, "detaching PID from activation state");
        activations.detach(self.pid);

        // Only write back if there are still attached PIDs or the executive is running.
        // The executive will handle full cleanup when the last PID detaches.
        if !activations.attached_pids_is_empty() || activations.executive_running() {
            write_activations_json(&activations, &activations_json_path, lock)?;
        } else {
            // No PIDs and no executive - just drop the lock.
            // The state will be cleaned up naturally.
            drop(lock);
        }

        Ok(())
    }
}
