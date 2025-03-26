use anyhow::Result;
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::providers::services::{ProcessStates, stop_services};
use tracing::instrument;

use crate::commands::services::{ServicesEnvironment, guard_service_commands_available};
use crate::commands::{EnvironmentSelect, environment_select};
use crate::utils::message;
use crate::{environment_subcommand_metric, subcommand_metric};

#[derive(Bpaf, Debug, Clone)]
pub struct Stop {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    /// Names of the services to stop
    #[bpaf(positional("name"))]
    names: Vec<String>,
}

impl Stop {
    #[instrument(name = "stop", skip_all)]
    pub async fn handle(self, flox: Flox) -> Result<()> {
        environment_subcommand_metric!("services::stop", self.environment);

        let env = ServicesEnvironment::from_environment_selection(&flox, &self.environment)?;
        guard_service_commands_available(&env, &flox.system)?;

        let socket = env.socket();

        let processes = ProcessStates::read(socket)?;
        let named_processes = super::processes_by_name_or_default_to_all(
            &processes,
            &env.manifest.services,
            &flox.system,
            &self.names,
        )?;

        for process in named_processes {
            if !process.is_running {
                message::warning(format!("Service '{}' is not running", process.name));
                continue;
            }

            if let Err(err) = stop_services(socket, &[&process.name]) {
                message::error(format!(
                    "Failed to stop service '{}': {}",
                    process.name, err
                ));
                continue;
            }

            message::updated(format!("Service '{}' stopped", process.name));
        }

        Ok(())
    }
}
