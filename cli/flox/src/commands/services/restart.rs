use anyhow::Result;
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::providers::services::{restart_service, ProcessStates};
use tracing::instrument;

use super::supported_environment;
use crate::commands::{environment_select, EnvironmentSelect};
use crate::subcommand_metric;
use crate::utils::message;

#[derive(Bpaf, Debug, Clone)]
pub struct Restart {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    /// Names of the services to restart
    #[bpaf(positional("name"))]
    names: Vec<String>,
}

impl Restart {
    #[instrument(name = "restart", skip_all)]
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("services::restart");

        let env = supported_environment(&flox, self.environment)?;
        let socket = env.services_socket_path(&flox)?;

        let processes = ProcessStates::read(&socket)?;
        let named_processes = super::processes_by_name_or_default_to_all(&processes, &self.names)?;

        for process in named_processes {
            if let Err(err) = restart_service(&socket, &process.name) {
                message::error(format!(
                    "Failed to restart service '{}': {}",
                    process.name, err
                ));
                continue;
            }

            message::updated(format!("Service '{}' restarted", process.name));
        }

        Ok(())
    }
}
