use anyhow::{anyhow, Result};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::providers::services::{restart_service, ProcessStates};
use indoc::indoc;
use tracing::instrument;

use super::supported_concrete_environment;
use crate::commands::{
    activated_environments,
    environment_select,
    EnvironmentSelect,
    UninitializedEnvironment,
};
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

        let concrete_environment = supported_concrete_environment(&flox, &self.environment)?;
        let activated_environments = activated_environments();

        if !activated_environments.is_active(&UninitializedEnvironment::from_concrete_environment(
            &concrete_environment,
        )?) {
            return Err(anyhow!(indoc! {"
                Cannot restart services for an environment that is not activated.

                To activate and start services, run 'flox activate -s'
            "}));
        }

        let env = concrete_environment.into_dyn_environment();
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
