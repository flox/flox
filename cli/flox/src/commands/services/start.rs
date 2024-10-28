use std::io::stderr;
use std::path::Path;

use anyhow::{anyhow, Result};
use bpaf::Bpaf;
use flox_rust_sdk::data::System;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::manifest::ManifestServices;
use flox_rust_sdk::providers::services::{
    shutdown_process_compose_if_all_processes_stopped,
    start_service,
    ProcessStates,
};
use tracing::{debug, instrument};

use crate::commands::services::{
    guard_is_within_activation,
    guard_service_commands_available,
    start_services_with_new_process_compose,
    ServicesEnvironment,
};
use crate::commands::{environment_select, EnvironmentSelect};
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

        let env = ServicesEnvironment::from_environment_selection(&flox, &self.environment)?;
        guard_is_within_activation(&env, "start")?;
        guard_service_commands_available(&env, &flox.system)?;

        let start_new_process_compose = if !env.expect_services_running() {
            true
        } else {
            // Returns `Ok(true)` if `process-compose` was shutdown
            shutdown_process_compose_if_all_processes_stopped(env.socket())?
        };

        if start_new_process_compose {
            debug!("starting services in new process-compose instance");
            let names = start_services_with_new_process_compose(
                config,
                flox,
                self.environment,
                env.into_inner(),
                &self.names,
            )
            .await?;
            for name in names {
                message::updated(format!("Service '{name}' started."));
            }
            Ok(())
        } else {
            debug!("starting services with existing process-compose instance");
            Self::start_with_existing_process_compose(
                env.socket(),
                &env.manifest.services,
                &flox.system,
                &self.names,
                &mut stderr(),
            )
        }
    }

    /// Starts services using an already running process-compose.
    /// Defaults to starting all services if no services are specified.
    fn start_with_existing_process_compose(
        socket: impl AsRef<Path>,
        manifest_services: &ManifestServices,
        system: impl Into<System>,
        names: &[String],
        err_stream: &mut impl std::io::Write,
    ) -> Result<()> {
        let processes = ProcessStates::read(&socket)?;
        let named_processes = super::processes_by_name_or_default_to_all(
            &processes,
            manifest_services,
            system,
            names,
        )?;

        let mut failure_count = 0;
        for process in named_processes {
            if process.is_running {
                message::warning_to_buffer(
                    err_stream,
                    format!("Service '{}' is already running.", process.name),
                );
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::io;

    use flox_rust_sdk::providers::services::test_helpers::TestProcessComposeInstance;
    use flox_rust_sdk::providers::services::{generate_never_exit_process, ProcessComposeConfig};

    use super::*;

    /// start_with_existing_process_compose errors when called with a nonexistent service
    #[test]
    fn error_starting_nonexistent_service_with_existing_process_compose() {
        let instance = TestProcessComposeInstance::start(&ProcessComposeConfig {
            processes: BTreeMap::new(),
            ..Default::default()
        });

        let err = Start::start_with_existing_process_compose(
            instance.socket(),
            &Default::default(),
            "system",
            &["one".to_string()],
            &mut io::stderr(),
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("Service 'one' does not exist."),
            "{err}"
        );
    }

    /// start_with_existing_process_compose can start a specified service
    #[test]
    fn start_specified_service() {
        let instance = TestProcessComposeInstance::start_services(
            &ProcessComposeConfig {
                processes: [
                    ("one".to_string(), generate_never_exit_process()),
                    ("two".to_string(), generate_never_exit_process()),
                ]
                .into(),
                ..Default::default()
            },
            &["one".to_string()],
        );

        let states = ProcessStates::read(instance.socket()).unwrap();
        let one_state = states.process("one").unwrap();
        assert!(one_state.is_running);
        let two_state = states.process("two").unwrap();
        assert!(!two_state.is_running);

        Start::start_with_existing_process_compose(
            instance.socket(),
            &Default::default(),
            "system",
            &["two".to_string()],
            &mut io::stderr(),
        )
        .unwrap();
        let states = ProcessStates::read(instance.socket()).unwrap();
        let one_state = states.process("one").unwrap();
        assert!(one_state.is_running);
        let two_state = states.process("two").unwrap();
        assert!(two_state.is_running);
    }

    /// start_with_existing_process_compose defaults to starting all services
    /// and warns for already started services
    #[test]
    fn start_defaults_to_all_services() {
        let instance = TestProcessComposeInstance::start_services(
            &ProcessComposeConfig {
                processes: [
                    ("one".to_string(), generate_never_exit_process()),
                    ("two".to_string(), generate_never_exit_process()),
                    ("three".to_string(), generate_never_exit_process()),
                ]
                .into(),
                ..Default::default()
            },
            &["one".to_string()],
        );

        let states = ProcessStates::read(instance.socket()).unwrap();
        let one_state = states.process("one").unwrap();
        assert!(one_state.is_running);
        let two_state = states.process("two").unwrap();
        assert!(!two_state.is_running);
        let three_state = states.process("three").unwrap();
        assert!(!three_state.is_running);

        let mut out = Vec::new();
        Start::start_with_existing_process_compose(
            instance.socket(),
            &Default::default(),
            "system",
            &[],
            &mut out,
        )
        .unwrap();
        let states = ProcessStates::read(instance.socket()).unwrap();
        let one_state = states.process("one").unwrap();
        assert!(one_state.is_running);
        let two_state = states.process("two").unwrap();
        assert!(two_state.is_running);
        let three_state = states.process("three").unwrap();
        assert!(three_state.is_running);

        let out = String::from_utf8(out).unwrap();
        assert_eq!(out, "⚠️  Service 'one' is already running.\n");
    }
}
