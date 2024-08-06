use std::path::Path;

use anyhow::{anyhow, Result};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::lockfile::LockedManifest;
use flox_rust_sdk::providers::services::{process_compose_down, start_service, ProcessStates};
use indoc::indoc;
use tracing::{debug, instrument};

use crate::commands::activate::Activate;
use crate::commands::services::{service_does_not_exist_error, supported_concrete_environment};
use crate::commands::{
    activated_environments,
    environment_select,
    ConcreteEnvironment,
    EnvironmentSelect,
    UninitializedEnvironment,
};
use crate::config::Config;
use crate::subcommand_metric;
use crate::utils::message;

#[derive(Bpaf, Debug, Clone)]
pub struct Start {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    /// Names of the services to start
    #[bpaf(positional("name"))]
    names: Vec<String>,
}

impl Start {
    #[instrument(name = "start", skip_all)]
    pub async fn handle(self, config: Config, flox: Flox) -> Result<()> {
        subcommand_metric!("services::start");

        let mut concrete_environment = supported_concrete_environment(&flox, &self.environment)?;
        let activated_environments = activated_environments();

        if !activated_environments.is_active(&UninitializedEnvironment::from_concrete_environment(
            &concrete_environment,
        )?) {
            return Err(anyhow!(indoc! {"
                Cannot start services for an environment that is not activated.

                To activate and start services, run 'flox activate -s'
            "}));
        }

        // TODO: this doesn't need to be mut
        let env = concrete_environment.dyn_environment_ref_mut();
        let socket = env.services_socket_path(&flox)?;

        let start_new_process_compose = if !socket.exists() {
            true
        } else {
            let processes = ProcessStates::read(&socket)?;
            let all_processes_stopped = processes.iter().all(|p| p.is_stopped());
            all_processes_stopped
        };

        if start_new_process_compose {
            debug!("starting services in new process-compose instance");
            self.start_with_new_process_compose(config, flox, concrete_environment)
                .await
        } else {
            debug!("starting services with existing process-compose instance");
            Self::start_with_existing_process_compose(socket, &self.names)
        }
    }

    /// Note that this must be called within an existing activation, otherwise it
    /// will leave behind a process-compose since it doesn't start a watchdog.
    async fn start_with_new_process_compose(
        &self,
        config: Config,
        flox: Flox,
        mut concrete_environment: ConcreteEnvironment,
    ) -> Result<()> {
        let environment = concrete_environment.dyn_environment_ref_mut();
        let lockfile = environment.lockfile(&flox)?;
        let LockedManifest::Catalog(lockfile) = lockfile else {
            unreachable!("at least it should be after https://github.com/flox/flox/issues/1858")
        };
        for name in &self.names {
            if !lockfile.manifest.services.contains_key(name) {
                return Err(service_does_not_exist_error(name));
            }
        }
        Activate {
            environment: self.environment.clone(),
            trust: false,
            print_script: false,
            start_services: true,
            run_args: vec!["true".to_string()],
        }
        .activate(config, flox, concrete_environment, false, &self.names)
        .await?;
        // We don't know if the service actually started because we don't have
        // healthchecks.
        // But we do know that activate blocks until `process-compose` is running.
        // mytodo: test
        if self.names.is_empty() {
            for name in lockfile.manifest.services.keys() {
                message::updated(format!("Service '{name}' started."));
            }
        } else {
            for name in &self.names {
                message::updated(format!("Service '{name}' started."));
            }
        }
        Ok(())
    }

    // Starts services using an already running process-compose.
    // Defaults to starting all services if no services are specified.
    fn start_with_existing_process_compose(
        socket: impl AsRef<Path>,
        names: &[String],
    ) -> Result<()> {
        let processes = ProcessStates::read(&socket)?;
        let named_processes = super::processes_by_name_or_default_to_all(&processes, names)?;

        let mut failure_count = 0;
        for process in named_processes {
            if process.is_running {
                message::warning(format!("Service '{}' is running.", process.name));
                continue;
            }

            match start_service(&socket, &process.name) {
                Ok(_) => {
                    message::updated(format!("Service '{}' started.", process.name));
                },
                Err(e) => {
                    message::error(format!("Failed to start service '{}': {}", process.name, e));
                    failure_count += 1;
                },
            }
        }

        if failure_count > 0 {
            return Err(anyhow!("Failed to start {} services.", failure_count));
        }
        Ok(())
    }
}
