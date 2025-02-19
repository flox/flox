use std::cmp::max;
use std::fmt::Display;

use anyhow::{anyhow, Result};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::manifest::typed::Inner;
use flox_rust_sdk::providers::services::{LoggedError, ProcessState, ProcessStates, ServiceError};
use itertools::Itertools;
use serde::Serialize;
use tracing::instrument;

use crate::commands::services::{guard_service_commands_available, ServicesEnvironment};
use crate::commands::{environment_select, EnvironmentSelect};
use crate::{environment_subcommand_metric, subcommand_metric};

#[derive(Bpaf, Debug, Clone)]
pub struct Status {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    /// Display output as JSON
    #[bpaf(long)]
    json: bool,

    /// Names of the services to query
    #[bpaf(positional("name"))]
    names: Vec<String>,
}

impl Status {
    #[instrument(name = "status", skip_all)]
    pub async fn handle(self, flox: Flox) -> Result<()> {
        environment_subcommand_metric!("services::status", self.environment);

        let env = ServicesEnvironment::from_environment_selection(&flox, &self.environment)?;
        guard_service_commands_available(&env, &flox.system)?;

        let processes = ProcessStates::read(env.socket());

        let process_states_display = match processes {
            // When services haven't been started, there's no socket yet. Rather than
            // print an error for `flox services status` we should display the process
            // statuses as not yet started.
            Err(ServiceError::LoggedError(LoggedError::SocketDoesntExist)) => {
                let mut states = vec![];
                let service_names = if self.names.is_empty() {
                    env.manifest
                        .services
                        .inner()
                        .keys()
                        .cloned()
                        .collect::<Vec<_>>()
                } else {
                    self.names.clone()
                };
                for service in service_names.into_iter() {
                    let state = ProcessStateDisplay {
                        name: service,
                        status: "Stopped".to_string(),
                        pid: None,
                        exit_code: None,
                        is_running: false,
                    };
                    states.push(state);
                }
                Ok(ProcessStatesDisplay(states))
            },
            // Successful retrieval of process statuses get passed through.
            Ok(processes) => {
                let named_processes = super::processes_by_name_or_default_to_all(
                    &processes,
                    &env.manifest.services,
                    &flox.system,
                    &self.names,
                )?;

                Ok(named_processes
                    .into_iter()
                    .cloned()
                    .collect::<ProcessStatesDisplay>())
            },
            // All other errors will be returned and handled.
            // The unwrapping and re-wrapping here is just to make the types
            // work out for the Ok(_) variant, which doesn't exist at this point.
            Err(err) => Err(anyhow!(err)),
        }?;

        if self.json {
            let json_array = serde_json::to_string_pretty(&process_states_display)?;
            println!("{json_array}");
        } else {
            println!("{process_states_display}");
        }

        Ok(())
    }
}

/// Simplified version of ProcessState for display in the CLI.
#[derive(Clone, Debug, Serialize)]
struct ProcessStateDisplay {
    name: String,
    status: String,
    pid: Option<u64>,
    exit_code: Option<i32>,
    #[serde(skip_serializing)]
    is_running: bool,
}

impl From<ProcessState> for ProcessStateDisplay {
    fn from(proc: ProcessState) -> Self {
        ProcessStateDisplay {
            name: proc.name,
            status: proc.status,
            pid: Some(proc.pid),
            // process-compose uses -1 to indicate
            // that the process was stopped _by process-compose_.
            // for running services, process-compose will set the exit code to 0,
            // so exit code alone cannot be used to determine if the process is running.
            // Neither case was a valid exit code, so we put `None` here.
            exit_code: if proc.is_running || proc.exit_code == -1 {
                None
            } else {
                Some(proc.exit_code)
            },
            is_running: proc.is_running,
        }
    }
}

impl ProcessStateDisplay {
    /// Formats the PID for display to indicate whether it's currently running.
    fn pid_display(&self) -> String {
        if let Some(pid) = self.pid {
            if self.is_running {
                pid.to_string()
            } else {
                format!("[{}]", pid)
            }
        } else {
            String::new()
        }
    }
}

/// Simplified version of ProcessStates for display in the CLI.
#[derive(Clone, Debug, Serialize)]
struct ProcessStatesDisplay(Vec<ProcessStateDisplay>);

impl From<ProcessStates> for ProcessStatesDisplay {
    fn from(procs: ProcessStates) -> Self {
        ProcessStatesDisplay::from_iter(procs)
    }
}
impl IntoIterator for ProcessStatesDisplay {
    type IntoIter = std::vec::IntoIter<ProcessStateDisplay>;
    type Item = ProcessStateDisplay;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl FromIterator<ProcessState> for ProcessStatesDisplay {
    fn from_iter<T: IntoIterator<Item = ProcessState>>(iter: T) -> Self {
        ProcessStatesDisplay(
            iter.into_iter()
                .sorted_by_key(|proc| proc.name.clone())
                .map(ProcessStateDisplay::from)
                .collect(),
        )
    }
}

/// Formats `ProcessStates` as a table for display in the CLI.
impl Display for ProcessStatesDisplay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fn display_status(proc: &ProcessStateDisplay) -> String {
            if let Some(exit_code) = proc.exit_code {
                format!("{} ({})", proc.status, exit_code)
            } else {
                proc.status.clone()
            }
        }

        // TODO: Use a table writer library if we add any more variable width calculations.
        let name_width_min = 10;
        let name_width = max(
            name_width_min,
            self.0.iter().map(|proc| proc.name.len()).max().unwrap_or(0),
        );

        let status_width = self
            .0
            .iter()
            .map(|proc| display_status(proc).len())
            .max()
            .unwrap_or(6);

        writeln!(
            f,
            "{:<name_width$} {:<status_width$} {:>8}",
            "NAME", "STATUS", "PID",
        )?;

        for proc in &self.0 {
            writeln!(
                f,
                "{:<name_width$} {:<status_width$} {:>8}",
                proc.name,
                display_status(proc),
                proc.pid_display(),
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use flox_rust_sdk::providers::services::test_helpers::{
        generate_completed_process_state,
        generate_process_state,
        generate_stopped_process_state,
    };
    use indoc::indoc;
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_processstatesdisplay_name_sorted() {
        let states = ProcessStates::from(vec![
            generate_process_state("bbb", "Running", 123, true),
            generate_process_state("zzz", "Running", 123, true),
            generate_process_state("aaa", "Running", 123, true),
            generate_process_state("ccc", "Running", 123, true),
        ]);
        let states_display: ProcessStatesDisplay = states.into();
        assert_eq!(format!("{states_display}"), indoc! {"
            NAME       STATUS       PID
            aaa        Running      123
            bbb        Running      123
            ccc        Running      123
            zzz        Running      123
        "});
    }

    #[test]
    fn test_processstatesdisplay_name_padded() {
        let states = ProcessStates::from(vec![
            generate_process_state("short", "Running", 123, true),
            generate_process_state("longlonglonglonglong", "Running", 123, true),
        ]);
        let states_display: ProcessStatesDisplay = states.into();
        assert_eq!(format!("{states_display}"), indoc! {"
            NAME                 STATUS       PID
            longlonglonglonglong Running      123
            short                Running      123
        "});
    }

    #[test]
    fn test_processstatesdisplay_status_variants() {
        let states = ProcessStates::from(vec![
            generate_process_state("aaa", "Running", 123, true),
            generate_stopped_process_state("bbb", 456),
            generate_completed_process_state("ccc", 789, 0),
        ]);
        let states_display: ProcessStatesDisplay = states.into();
        assert_eq!(format!("{states_display}"), indoc! {"
            NAME       STATUS             PID
            aaa        Running            123
            bbb        Stopped          [456]
            ccc        Completed (0)    [789]
        "});
    }

    #[test]
    fn test_processstatesdisplay_pid_aligned() {
        let states = ProcessStates::from(vec![
            generate_process_state("aaa", "Running", 1, true),
            generate_process_state("bbb", "Running", 12, true),
            generate_process_state("ccc", "Running", 123, true),
            generate_process_state("ddd", "Running", 1234, true),
            generate_process_state("eee", "Running", 12345, true),
        ]);
        let states_display: ProcessStatesDisplay = states.into();
        assert_eq!(format!("{states_display}"), indoc! {"
            NAME       STATUS       PID
            aaa        Running        1
            bbb        Running       12
            ccc        Running      123
            ddd        Running     1234
            eee        Running    12345
        "});
    }

    #[test]
    fn test_processstatesdisplay_json_array() {
        let states = ProcessStates::from(vec![
            generate_process_state("aaa", "Running", 123, true),
            generate_stopped_process_state("bbb", 456),
            generate_completed_process_state("ccc", 789, 0),
        ]);
        let states_display: ProcessStatesDisplay = states.into();
        let json_array = serde_json::to_string_pretty(&states_display).unwrap();
        assert_eq!(json_array, indoc! {r#"[
              {
                "name": "aaa",
                "status": "Running",
                "pid": 123,
                "exit_code": null
              },
              {
                "name": "bbb",
                "status": "Stopped",
                "pid": 456,
                "exit_code": null
              },
              {
                "name": "ccc",
                "status": "Completed",
                "pid": 789,
                "exit_code": 0
              }
            ]"#});
    }
}
