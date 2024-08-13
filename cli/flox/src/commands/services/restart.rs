use std::path::Path;

use anyhow::{anyhow, Result};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::providers::services::{process_compose_down, restart_service, ProcessStates};
use tracing::{debug, instrument};

use crate::commands::services::{
    start_with_new_process_compose,
    supported_concrete_environment,
    ServicesCommandsError,
};
use crate::commands::{
    activated_environments,
    environment_select,
    EnvironmentSelect,
    UninitializedEnvironment,
};
use crate::config::Config;
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
    pub async fn handle(self, config: Config, flox: Flox) -> Result<()> {
        subcommand_metric!("services::restart");

        let concrete_environment = supported_concrete_environment(&flox, &self.environment)?;
        let activated_environments = activated_environments();

        if !activated_environments.is_active(&UninitializedEnvironment::from_concrete_environment(
            &concrete_environment,
        )?) {
            return Err(ServicesCommandsError::NotInActivation {
                action: "restart".to_string(),
            }
            .into());
        }

        let env = concrete_environment.dyn_environment_ref();
        let socket = env.services_socket_path(&flox)?;

        let start_new_process_compose = if !socket.exists() {
            true
        } else if self.names.is_empty() {
            // TODO: We could optimise by checking whether the manifest has actually changed.
            process_compose_down(&socket)?;
            true
        } else {
            let processes = ProcessStates::read(&socket)?;
            let all_processes_stopped = processes.iter().all(|p| p.is_stopped());
            if all_processes_stopped {
                process_compose_down(&socket)?;
            }
            all_processes_stopped
        };

        if start_new_process_compose {
            debug!("restarting services in new process-compose instance");
            let names = start_with_new_process_compose(
                config,
                flox,
                self.environment,
                concrete_environment,
                &self.names,
            )
            .await?;
            for name in names {
                message::updated(format!("Service '{name}' restarted."));
            }
            Ok(())
        } else {
            debug!("restarting services with existing process-compose instance");
            Self::restart_with_existing_process_compose(socket, &self.names)
        }
    }

    // Retarts services using an already running process-compose.
    // Defaults to restarting all services if no services are specified.
    fn restart_with_existing_process_compose(
        socket: impl AsRef<Path>,
        names: &[String],
    ) -> Result<()> {
        let processes = ProcessStates::read(&socket)?;
        let named_processes = super::processes_by_name_or_default_to_all(&processes, names)?;

        let mut failure_count = 0;
        for process in named_processes {
            match restart_service(&socket, &process.name) {
                Ok(_) => {
                    message::updated(format!("Service '{}' restarted.", process.name));
                },
                Err(e) => {
                    message::error(format!(
                        "Failed to restart service '{}': {}",
                        process.name, e
                    ));
                    failure_count += 1;
                },
            }
        }

        if failure_count > 0 {
            return Err(anyhow!("Failed to restart {} services.", failure_count));
        }
        Ok(())
    }
}
