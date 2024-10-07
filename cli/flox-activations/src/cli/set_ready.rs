use std::path::PathBuf;

use anyhow::Context;
use clap::Args;
use uuid::Uuid;

use crate::activations;

type Error = anyhow::Error;

#[derive(Debug, Args)]
pub struct SetReadyArgs {
    #[arg(help = "The path to the .flox directory for the environment.")]
    #[arg(short, long, value_name = "PATH")]
    flox_env: PathBuf,
    #[arg(help = "The UUID for this particular activation of this environment.")]
    #[arg(short, long, value_name = "UUID")]
    id: Uuid,
}

impl SetReadyArgs {
    pub(crate) fn handle(self, cache_dir: PathBuf) -> Result<(), Error> {
        let activations_json_path = activations::activations_json_path(&cache_dir, &self.flox_env)?;

        let (activations, lock) = activations::read_activations_json(&activations_json_path)?;
        let Some(mut activations) = activations else {
            anyhow::bail!("Expected an existing activations.json file");
        };

        let activation = activations
            .activation_for_id_mut(self.id)
            .with_context(|| {
                format!(
                    "No activation with ID {} found for environment {}",
                    self.id,
                    self.flox_env.display()
                )
            })?;

        activation.set_ready();

        activations::write_activations_json(&activations, &activations_json_path, lock)?;

        Ok(())
    }
}
