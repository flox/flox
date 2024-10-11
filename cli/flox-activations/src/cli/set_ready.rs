use std::path::PathBuf;

use anyhow::Context;
use clap::Args;

use crate::activations;

type Error = anyhow::Error;

#[derive(Debug, Args)]
pub struct SetReadyArgs {
    #[arg(help = "The path to the .flox directory for the environment.")]
    #[arg(short, long, value_name = "PATH")]
    flox_env: PathBuf,
    #[arg(help = "The ID for this particular activation of this environment.")]
    #[arg(short, long, value_name = "ID")]
    id: String,
}

impl SetReadyArgs {
    pub(crate) fn handle(self, runtime_dir: PathBuf) -> Result<(), Error> {
        let activations_json_path =
            activations::activations_json_path(&runtime_dir, &self.flox_env)?;

        let (activations, lock) = activations::read_activations_json(&activations_json_path)?;
        let Some(mut activations) = activations else {
            anyhow::bail!("Expected an existing activations.json file");
        };

        let activation = activations
            .activation_for_id_mut(&self.id)
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

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::cli::test::{read_activations, write_activations};

    #[test]
    fn set_ready() {
        let runtime_dir = TempDir::new().unwrap();
        let flox_env = PathBuf::from("/path/to/floxenv");
        let pid = 5678;

        let id = write_activations(&runtime_dir, &flox_env, |activations| {
            activations
                .create_activation("/store/path", pid)
                .unwrap()
                .id()
        });

        let ready = read_activations(&runtime_dir, &flox_env, |activations| {
            activations.activation_for_id_ref(&id).unwrap().ready()
        })
        .unwrap();

        assert!(!ready);

        let args = SetReadyArgs {
            flox_env: flox_env.clone(),
            id: id.clone(),
        };

        args.handle(runtime_dir.path().to_path_buf()).unwrap();

        let ready = read_activations(&runtime_dir, &flox_env, |activations| {
            activations.activation_for_id_ref(id).unwrap().ready()
        })
        .unwrap();

        assert!(ready);
    }
}
