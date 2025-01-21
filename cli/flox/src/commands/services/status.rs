use std::cmp::max;
use std::fmt::Display;

use anyhow::Result;
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::providers::services::{ProcessState, ProcessStates};
use itertools::Itertools;
use serde::Serialize;
use tracing::instrument;

use crate::commands::services::{guard_service_commands_available, ServicesEnvironment};
use crate::commands::{environment_select, EnvironmentSelect};
use crate::subcommand_metric;

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
        subcommand_metric!("services::status");

        let env = ServicesEnvironment::from_environment_selection(&flox, &self.environment)?;
        guard_service_commands_available(&env, &flox.system)?;

        let processes = ProcessStates::read(env.socket())?;

        let named_processes = super::processes_by_name_or_default_to_all(
            &processes,
            &env.manifest.services,
            &flox.system,
            &self.names,
        )?;

        let process_states_display = named_processes
            .into_iter()
            .cloned()
            .collect::<ProcessStatesDisplay>();

        if self.json {
            for proc in process_states_display {
                let line = serde_json::to_string(&proc)?;
                println!("{line}");
            }
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
    pid: u64,
    exit_code: Option<i32>,
    #[serde(skip_serializing)]
    is_running: bool,
}

impl From<ProcessState> for ProcessStateDisplay {
    fn from(proc: ProcessState) -> Self {
        ProcessStateDisplay {
            name: proc.name,
            status: proc.status,
            pid: proc.pid,
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
        if self.is_running {
            self.pid.to_string()
        } else {
            format!("[{}]", self.pid)
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
    use std::io::Write;

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
    fn test_processstatedisplay_json_lines() {
        let states = ProcessStates::from(vec![
            generate_process_state("aaa", "Running", 123, true),
            generate_stopped_process_state("bbb", 456),
            generate_completed_process_state("ccc", 789, 0),
        ]);
        let states_display: ProcessStatesDisplay = states.into();
        let mut buffer = Vec::new();
        for proc in states_display {
            let line = serde_json::to_string(&proc).unwrap();
            writeln!(buffer, "{line}").unwrap();
        }
        let buffer_str = String::from_utf8(buffer).unwrap();
        assert_eq!(buffer_str, indoc! {r#"
            {"name":"aaa","status":"Running","pid":123,"exit_code":null}
            {"name":"bbb","status":"Stopped","pid":456,"exit_code":null}
            {"name":"ccc","status":"Completed","pid":789,"exit_code":0}
        "#});
    }
}
