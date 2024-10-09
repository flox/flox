use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::Args;
use indoc::indoc;
use time::{Duration, OffsetDateTime};

use crate::activations;

#[derive(Debug, Args)]
pub struct StartOrAttachArgs {
    #[arg(help = "The PID of the shell registering interest in the activation.")]
    #[arg(short, long, value_name = "PID")]
    pub pid: u32,
    #[arg(help = "The path to the .flox directory for the environment.")]
    #[arg(short, long, value_name = "PATH")]
    pub flox_env: PathBuf,
    #[arg(help = "The store path of the rendered environment for this activation.")]
    #[arg(short, long, value_name = "PATH")]
    pub store_path: String,
}

impl StartOrAttachArgs {
    pub(crate) fn handle(self, cache_dir: PathBuf) -> Result<(), anyhow::Error> {
        let activations_json_path = activations::activations_json_path(&cache_dir, &self.flox_env)?;

        let (activations, lock) = activations::read_activations_json(&activations_json_path)?;
        let mut activations = activations.unwrap_or_default();

        let (activation_id, attaching) = match activations
            .activation_for_store_path(&self.store_path)
        {
            Some(activation) => {
                let activation_id = activation.id();

                let attach_expiration = OffsetDateTime::now_utc() + Duration::seconds(10);

                // While we wait for the activation to become ready,
                // we need to drop the lock to allow other processes to write to the file.
                drop(lock);
                wait_for_activation_ready_and_attach_pid(
                    &activations_json_path,
                    activation_id,
                    attach_expiration,
                    self.pid,
                )?;

                (activation_id, true)
            },
            None => {
                let activation_id = activations
                    .create_activation(&self.store_path, self.pid)?
                    .id();

                activations::write_activations_json(&activations, &activations_json_path, lock)?;

                (activation_id, false)
            },
        };

        println!("_FLOX_ATTACH={attaching}");
        println!(
            "_FLOX_ACTIVATION_STATE_DIR={}",
            activations::activation_state_dir_path(cache_dir, self.flox_env, activation_id)?
                .display()
        );
        println!("_FLOX_ACTIVATION_ID={activation_id}");

        Ok(())
    }
}

/// Wait for the activation with the given ID to become ready.
/// I.e if an activation is being started already, wait for it to become ready,
/// then _attach_ the given PID to it.
/// If the activation is not ready within the given timeout,
/// exit with an error.
/// If the activation startup process fails, exit with an error.
/// In either case, the activation can likely just be restarted.
// TODO: use store_path rather than activation_id.
// If we are activation 3, activation 1 fails, but activation 2 succeeds, we may want to attach to activation 2
fn wait_for_activation_ready_and_attach_pid(
    activations_json_path: &Path,
    activation_id: uuid::Uuid,
    attach_expiration: OffsetDateTime,
    attaching_pid: u32,
) -> Result<(), anyhow::Error> {
    loop {
        let (activations, lock) = activations::read_activations_json(activations_json_path)?;
        let Some(mut activations) = activations else {
            anyhow::bail!("Expected an existing activations.json file");
        };

        let activation = activations
            .activation_for_id_mut(activation_id)
            .context(indoc! {"
                Prior activation of the environment completed.

                Try again to start a new activation of the environment.
            "})?;

        if activation.ready() {
            activation.attach_pid(attaching_pid, None);
            activations::write_activations_json(&activations, activations_json_path, lock)?;
            break;
        }

        if !activation.startup_process_running() {
            // TODO: clean out old activation of store_path
            // Or we may need to do that in activation_for_store_path()
            //
            // TODO: just call StartOrAttach::handle again
            anyhow::bail!(indoc! {"
                Prior activation of the environment failed to start, or completed.

                Try again to start a new activation of the environment.
            "});
        }

        if OffsetDateTime::now_utc() > attach_expiration {
            anyhow::bail!(indoc! {"
                Timed out waiting for a prior activation of the environment
                to complete startup hooks.

                Try again after the previous activation of the environment has completed.
            "});
        }

        // Drop the lock to allow the startup process to write to the file.
        drop(lock);
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Ok(())
}
