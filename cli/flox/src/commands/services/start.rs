use std::io::stderr;
use std::path::Path;

use anyhow::{anyhow, Result};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::providers::services::{process_compose_down, start_service, ProcessStates};
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
            if all_processes_stopped {
                process_compose_down(&socket)?;
            }
            all_processes_stopped
        };

        if start_new_process_compose {
            debug!("starting services in new process-compose instance");
            let names = start_with_new_process_compose(
                config,
                flox,
                self.environment,
                concrete_environment,
                &self.names,
            )
            .await?;
            for name in names {
                message::updated(format!("Service '{name}' started."));
            }
            Ok(())
        } else {
            debug!("starting services with existing process-compose instance");
            Self::start_with_existing_process_compose(socket, &self.names, &mut stderr())
        }
    }

    // Starts services using an already running process-compose.
    // Defaults to starting all services if no services are specified.
    fn start_with_existing_process_compose(
        socket: impl AsRef<Path>,
        names: &[String],
        err_stream: &mut impl std::io::Write,
    ) -> Result<()> {
        let processes = ProcessStates::read(&socket)?;
        let named_processes = super::processes_by_name_or_default_to_all(&processes, names)?;

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
    use flox_rust_sdk::providers::services::{ProcessComposeConfig, ProcessConfig};

    use super::*;

    /// start_with_existing_process_compose errors when called with a nonexistent service
    #[test]
    fn error_starting_nonexistent_service_with_existing_process_compose() {
        let instance = TestProcessComposeInstance::start(&ProcessComposeConfig {
            processes: BTreeMap::new(),
        });

        let err = Start::start_with_existing_process_compose(
            instance.socket(),
            &["one".to_string()],
            &mut io::stderr(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("Service 'one' not found."));
    }

    /// start_with_existing_process_compose can start a specified service
    #[test]
    fn start_specified_service() {
        let instance = TestProcessComposeInstance::start_services(
            &ProcessComposeConfig {
                processes: [
                    ("one".to_string(), ProcessConfig {
                        command: String::from("sleep infinity"),
                        vars: None,
                    }),
                    ("two".to_string(), ProcessConfig {
                        command: String::from("sleep infinity"),
                        vars: None,
                    }),
                ]
                .into(),
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
                    ("one".to_string(), ProcessConfig {
                        command: String::from("sleep infinity"),
                        vars: None,
                    }),
                    ("two".to_string(), ProcessConfig {
                        command: String::from("sleep infinity"),
                        vars: None,
                    }),
                    ("three".to_string(), ProcessConfig {
                        command: String::from("sleep infinity"),
                        vars: None,
                    }),
                ]
                .into(),
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
        Start::start_with_existing_process_compose(instance.socket(), &[], &mut out).unwrap();
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
