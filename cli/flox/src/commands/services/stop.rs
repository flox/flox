use anyhow::Result;
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::providers::services::{stop_services, ProcessStates};
use tracing::instrument;

use super::supported_environment;
use crate::commands::{environment_select, EnvironmentSelect};
use crate::subcommand_metric;
use crate::utils::message;

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
        subcommand_metric!("services::stop");

        let env = supported_environment(&flox, self.environment)?;
        let socket = env.services_socket_path(&flox)?;

        let processes = ProcessStates::read(&socket)?;
        let named_processes = super::processes_by_name_or_default_to_all(&processes, &self.names)?;

        for process in named_processes {
            if !process.is_running {
                message::warning(format!("Service '{}' is not running", process.name));
                continue;
            }

            message::updated(format!("Service '{}' stopped", process.name));
        }

        Ok(())
    }
}
