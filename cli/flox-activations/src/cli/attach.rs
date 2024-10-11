use std::path::PathBuf;

use anyhow::Context;
use clap::Args;
use time::Duration;

use crate::{activations, Error};

#[derive(Debug, Args)]
pub struct AttachArgs {
    #[arg(help = "The PID of the shell registering interest in the activation.")]
    #[arg(short, long, value_name = "PID")]
    pub pid: u32,
    #[arg(help = "The path to the .flox directory for the environment.")]
    #[arg(short, long, value_name = "PATH")]
    pub flox_env: PathBuf,
    #[arg(help = "The ID for this particular activation of this environment.")]
    #[arg(short, long, value_name = "ID")]
    pub id: String,
    #[command(flatten)]
    pub exclusive: AttachExclusiveArgs,
}

#[derive(Debug, Args)]
#[group(required = true, multiple = false)]
pub struct AttachExclusiveArgs {
    #[arg(help = "How long to wait between termination of this PID and cleaning up its interest.")]
    #[arg(short, long, value_name = "TIME_MS")]
    pub timeout_ms: Option<u32>,
    #[arg(help = "Remove the specified PID when attaching to this activation.")]
    #[arg(short, long, value_name = "PID")]
    pub remove_pid: Option<u32>,
}

impl AttachArgs {
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

        match self.exclusive {
            AttachExclusiveArgs {
                timeout_ms: Some(timeout_ms),
                remove_pid: None,
            } => {
                activation.attach_pid(self.pid, Some(Duration::milliseconds(timeout_ms as i64)));
            },
            AttachExclusiveArgs {
                timeout_ms: None,
                remove_pid: Some(remove_pid),
            } => {
                activation.remove_pid(remove_pid);
                activation.attach_pid(self.pid, None)
            },
            // This should be unreachable due to the group constraints when constructed by clap
            _ => {
                anyhow::bail!("Exactly one of --timeout-ms or --remove-pid must be specified");
            },
        }

        activations::write_activations_json(&activations, &activations_json_path, lock)?;

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use std::path::PathBuf;

    use tempfile::TempDir;

    use super::{AttachArgs, AttachExclusiveArgs};
    use crate::activations::AttachedPid;
    use crate::cli::test::{read_activations, write_activations};

    #[test]
    fn attach_to_id_with_new_pid() {
        let runtime_dir = TempDir::new().unwrap();
        let flox_env = PathBuf::from("/path/to/floxenv");
        let new_pid = 5678;

        let id = write_activations(&runtime_dir, &flox_env, |activations| {
            activations
                .create_activation("/store/path", 1234)
                .unwrap()
                .id()
        });

        let args = AttachArgs {
            flox_env: flox_env.clone(),
            id: id.clone(),
            pid: new_pid,
            exclusive: AttachExclusiveArgs {
                timeout_ms: Some(1000),
                remove_pid: None,
            },
        };

        args.handle(runtime_dir.path().to_path_buf()).unwrap();

        let activation = read_activations(&runtime_dir, &flox_env, |activations| {
            activations.activation_for_id_ref(id).unwrap().clone()
        })
        .unwrap();

        activation
            .attached_pids()
            .iter()
            .find(|pid| pid.pid == new_pid)
            .expect("pid was attached");
    }

    #[test]
    fn attach_to_id_with_replace() {
        let runtime_dir = TempDir::new().unwrap();
        let flox_env = PathBuf::from("/path/to/floxenv");
        let old_pid = 1234;
        let new_pid = 5678;

        let id = write_activations(&runtime_dir, &flox_env, |activations| {
            activations
                .create_activation("/store/path", old_pid)
                .unwrap()
                .id()
        });

        let args = AttachArgs {
            flox_env: flox_env.clone(),
            id: id.clone(),
            pid: new_pid,
            exclusive: AttachExclusiveArgs {
                timeout_ms: None,
                remove_pid: Some(old_pid),
            },
        };

        args.handle(runtime_dir.path().to_path_buf()).unwrap();

        let activation = read_activations(&runtime_dir, &flox_env, |activations| {
            activations.activation_for_id_ref(id).unwrap().clone()
        })
        .unwrap();

        assert_eq!(activation.attached_pids(), &[AttachedPid {
            pid: new_pid,
            expiration: None
        }]);
    }
}
