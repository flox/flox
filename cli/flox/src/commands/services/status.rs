use std::cmp::max;
use std::fmt::Display;

use anyhow::Result;
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::providers::services::{ProcessState, ProcessStates};
use itertools::Itertools;
use serde::Serialize;
use tracing::instrument;

use crate::commands::services::{
    guard_service_commands_available,
    handle_service_connection_error,
    ServicesEnvironment,
};
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
        guard_service_commands_available(&env)?;

        let socket = env.socket();
        let processes = ProcessStates::read(socket)
            .map_err(|err| handle_service_connection_error(err, socket))?;

        let named_processes = super::processes_by_name_or_default_to_all(&processes, &self.names)?;

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
    #[serde(skip_serializing)]
    is_running: bool,
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
                .map(|proc| ProcessStateDisplay {
                    name: proc.name,
                    status: proc.status,
                    pid: proc.pid,
                    is_running: proc.is_running,
                })
                .collect(),
        )
    }
}

/// Formats `ProcessStates` as a table for display in the CLI.
impl Display for ProcessStatesDisplay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // TODO: Use a table writer library if we add any more variable width calculations.
        let name_width_min = 10;
        let name_width = max(
            name_width_min,
            self.0.iter().map(|proc| proc.name.len()).max().unwrap_or(0),
        );
        // Max value based on the possible states in:
        // https://github.com/F1bonacc1/process-compose/blob/v1.9.0/src/types/process.go#L125-L137
        let status_width = 12;

        writeln!(
            f,
            "{:<name_width$} {:<status_width$} {:>8}",
            "NAME", "STATUS", "PID"
        )?;
        for proc in &self.0 {
            writeln!(
                f,
                "{:<name_width$} {:<status_width$} {:>8}",
                proc.name,
                proc.status,
                proc.pid_display()
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use flox_rust_sdk::providers::services::test_helpers::generate_process_state;
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
            NAME       STATUS            PID
            aaa        Running           123
            bbb        Running           123
            ccc        Running           123
            zzz        Running           123
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
            NAME                 STATUS            PID
            longlonglonglonglong Running           123
            short                Running           123
        "});
    }

    #[test]
    fn test_processstatesdisplay_status_variants() {
        let states = ProcessStates::from(vec![
            generate_process_state("aaa", "Running", 123, true),
            generate_process_state("bbb", "Stopped", 123, false),
            generate_process_state("ccc", "Completed", 123, false),
        ]);
        let states_display: ProcessStatesDisplay = states.into();
        assert_eq!(format!("{states_display}"), indoc! {"
            NAME       STATUS            PID
            aaa        Running           123
            bbb        Stopped         [123]
            ccc        Completed       [123]
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
            NAME       STATUS            PID
            aaa        Running             1
            bbb        Running            12
            ccc        Running           123
            ddd        Running          1234
            eee        Running         12345
        "});
    }

    #[test]
    fn test_processstatedisplay_json_lines() {
        let states = ProcessStates::from(vec![
            generate_process_state("aaa", "Running", 123, true),
            generate_process_state("bbb", "Stopped", 123, false),
            generate_process_state("ccc", "Completed", 123, false),
        ]);
        let states_display: ProcessStatesDisplay = states.into();
        let mut buffer = Vec::new();
        for proc in states_display {
            let line = serde_json::to_string(&proc).unwrap();
            writeln!(buffer, "{line}").unwrap();
        }
        let buffer_str = String::from_utf8(buffer).unwrap();
        assert_eq!(buffer_str, indoc! {r#"
            {"name":"aaa","status":"Running","pid":123}
            {"name":"bbb","status":"Stopped","pid":123}
            {"name":"ccc","status":"Completed","pid":123}
        "#});
    }
}
