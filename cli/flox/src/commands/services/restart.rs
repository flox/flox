use std::path::Path;

use anyhow::{anyhow, Result};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::providers::services::{process_compose_down, restart_service, ProcessStates};
use indoc::indoc;
use tracing::{debug, instrument};

use crate::commands::services::{start_with_new_process_compose, supported_concrete_environment};
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

        let mut concrete_environment = supported_concrete_environment(&flox, &self.environment)?;
        let activated_environments = activated_environments();

        if !activated_environments.is_active(&UninitializedEnvironment::from_concrete_environment(
            &concrete_environment,
        )?) {
            return Err(anyhow!(indoc! {"
                Cannot restart services for an environment that is not activated.

                To activate and start services, run 'flox activate --start-services'
            "}));
        }

        // TODO: this doesn't need to be mut
        let env = concrete_environment.dyn_environment_ref_mut();
        let socket = env.services_socket_path(&flox)?;

        let existing_process_compose = socket.exists();
        let existing_processes = match existing_process_compose {
            true => ProcessStates::read(&socket)?,
            false => ProcessStates::from(vec![]),
        };
        let all_processes_stopped = existing_processes.iter().all(|p| p.is_stopped());
        let restart_all = self.names.is_empty();

        // TODO: We could optimise by checking whether the manifest has actually changed.
        let start_new_process_compose = restart_all || all_processes_stopped;

        if start_new_process_compose {
            if existing_process_compose {
                debug!("stopping existing process-compose instance");
                process_compose_down(&socket)?;
            }
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
                message::updated(format!(
                    "Service '{name}' {}.",
                    Self::action_for_service_name(&name, &existing_processes)
                ));
            }
            Ok(())
        } else {
            debug!("restarting services with existing process-compose instance");
            Self::restart_with_existing_process_compose(socket, &self.names, existing_processes)
        }
    }

    // Return a "started" or "restarted" action depending on whether the service
    // was previous consider running.
    fn action_for_service_name(name: &str, processes: &ProcessStates) -> String {
        let process = match processes.process(name) {
            Some(proc) => proc,
            None => return "started".to_string(),
        };
        match process.is_stopped() {
            true => "started".to_string(),
            false => "restarted".to_string(),
        }
    }

    // Retarts services using an already running process-compose.
    // Defaults to restarting all services if no services are specified.
    fn restart_with_existing_process_compose(
        socket: impl AsRef<Path>,
        names: &[String],
        processes: ProcessStates,
    ) -> Result<()> {
        let named_processes = super::processes_by_name_or_default_to_all(&processes, names)?;

        let mut failure_count = 0;
        for process in named_processes {
            match restart_service(&socket, &process.name) {
                Ok(_) => {
                    message::updated(format!(
                        "Service '{}' {}.",
                        process.name,
                        Self::action_for_service_name(&process.name, &processes),
                    ));
                },
                Err(e) => {
                    message::error(format!(
                        "Failed to {} service '{}': {}",
                        Self::action_for_service_name(&process.name, &processes),
                        process.name,
                        e
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
