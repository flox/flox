use std::path::Path;

use anyhow::{Result, anyhow};
use bpaf::Bpaf;
use flox_rust_sdk::data::System;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::manifest::typed::Services;
use flox_rust_sdk::providers::services::process_compose::{
    LoggedError,
    ProcessStates,
    ServiceError,
    restart_service,
};
use tracing::{debug, instrument};

use crate::commands::services::{
    ProcessComposeState,
    ServicesCommandsError,
    ServicesEnvironment,
    guard_is_within_activation,
    guard_service_commands_available,
    start_services_with_new_process_compose,
};
use crate::commands::{EnvironmentSelect, environment_select};
use crate::config::Config;
use crate::environment_subcommand_metric;
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
        let mut env = ServicesEnvironment::from_environment_selection(&flox, &self.environment)?;
        environment_subcommand_metric!("services::restart", env.environment);
        let (current_mode, generation) = guard_is_within_activation(&env, "restart")?;
        guard_service_commands_available(&env, &flox.system)?;

        let process_compose_state = env.process_compose_state(&flox, &current_mode);
        let socket = env.socket();

        let existing_processes = match ProcessStates::read(socket) {
            Ok(process_states) => process_states,
            Err(ServiceError::LoggedError(LoggedError::SocketDoesntExist)) => {
                ProcessStates::from(vec![])
            },
            Err(e) => return Err(e.into()),
        };

        let all_processes_stopped = existing_processes.iter().all(|p| p.is_stopped());
        let restart_all = self.names.is_empty();

        debug!(
            socket_exists = socket.exists(),
            ?process_compose_state,
            all_processes_stopped,
            restart_all,
            "evaluating restart conditions"
        );

        match process_compose_state {
            ProcessComposeState::ActivationStartingSelf => {
                Err(ServicesCommandsError::CalledFromActivationHook.into())
            },
            ProcessComposeState::NotCurrent if all_processes_stopped || restart_all => {
                debug!("restarting services in new process-compose instance");
                let names = start_services_with_new_process_compose(
                    config,
                    flox,
                    self.environment,
                    env.into_inner(),
                    current_mode,
                    &self.names,
                    generation,
                )
                .await?;
                for name in names {
                    message::updated(format!(
                        "Service '{name}' {}.",
                        Self::action_for_service_name(&name, &existing_processes)
                    ));
                }
                Ok(())
            },
            ProcessComposeState::Current | ProcessComposeState::NotCurrent => {
                debug!("restarting services with existing process-compose instance");
                Self::restart_with_existing_process_compose(
                    socket,
                    &env.manifest.services,
                    &flox.system,
                    &self.names,
                    existing_processes,
                )
            },
        }
    }

    // Return a "started" or "restarted" action depending on whether the service
    // was previously considered running.
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

    // Restarts services using an already running process-compose.
    // Defaults to restarting all services if no services are specified.
    fn restart_with_existing_process_compose(
        socket: impl AsRef<Path>,
        manifest_services: &Services,
        system: impl Into<System>,
        names: &[String],
        processes: ProcessStates,
    ) -> Result<()> {
        let named_processes = super::processes_by_name_or_default_to_all(
            &processes,
            manifest_services,
            system,
            names,
        )?;

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
