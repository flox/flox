use anyhow::{anyhow, Result};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::Environment;
use flox_rust_sdk::providers::services::{ProcessState, ProcessStates, ServiceError};
use tracing::instrument;

use super::{ConcreteEnvironment, EnvironmentSelect};

mod logs;
mod status;
mod stop;

/// Services Commands.
#[derive(Debug, Clone, Bpaf)]
pub enum ServicesCommands {
    /// Status of a service or services
    #[bpaf(command)]
    Status(#[bpaf(external(status::status))] status::Status),

    /// Stop a service or services
    #[bpaf(command)]
    Stop(#[bpaf(external(stop::stop))] stop::Stop),

    /// Print logs of services
    #[bpaf(command)]
    Logs(#[bpaf(external(logs::logs))] logs::Logs),
}

impl ServicesCommands {
    #[instrument(name = "services", skip_all)]
    pub async fn handle(self, flox: Flox) -> Result<()> {
        if !flox.features.services {
            return Err(ServiceError::FeatureFlagDisabled.into());
        }

        match self {
            ServicesCommands::Status(args) => args.handle(flox).await?,
            ServicesCommands::Stop(args) => args.handle(flox).await?,
            ServicesCommands::Logs(args) => args.handle(flox).await?,
        }

        Ok(())
    }
}

/// Return an Environment for variants that support services.
pub fn supported_environment(
    flox: &Flox,
    environment: EnvironmentSelect,
) -> Result<Box<dyn Environment>> {
    let concrete_environment = environment.detect_concrete_environment(flox, "Services in")?;
    if let ConcreteEnvironment::Remote(_) = concrete_environment {
        return Err(ServiceError::RemoteEnvsNotSupported.into());
    }
    let dyn_environment = concrete_environment.into_dyn_environment();
    Ok(dyn_environment)
}

/// Try to find processes by name, typically provided by the user via arguments,
/// or default to all processes.
///
/// If names are provided, all names must be names of actual services.
/// If an invalid name is provided, an error is returned.
fn processes_by_name_or_default_to_all<'a>(
    processes: &'a ProcessStates,
    names: &[String],
) -> Result<Vec<&'a ProcessState>> {
    if !names.is_empty() {
        names
            .iter()
            .map(|name| {
                processes
                    .process(name)
                    .ok_or_else(|| anyhow!("Service '{name}' not found"))
            })
            .collect::<Result<Vec<_>>>()
    } else {
        tracing::debug!("No service names provided, defaulting to all services");
        Ok(Vec::from_iter(processes.iter()))
    }
}

#[cfg(test)]
mod tests {
    use flox_rust_sdk::providers::services::test_helpers::generate_process_state;

    use super::*;

    #[test]
    fn processes_by_name_returns_named_processes() {
        let processes = [
            generate_process_state("foo", "Running", 123, true),
            generate_process_state("bar", "Completed", 123, false),
        ]
        .into();

        let all_processes = processes_by_name_or_default_to_all(&processes, &["foo".to_string()])
            .expect("naming 'foo' should return one process");

        assert_eq!(all_processes.len(), 1);
        assert_eq!(all_processes[0].name, "foo");
    }

    #[test]
    fn processes_by_name_returns_all_if_no_process_provided() {
        let processes = [
            generate_process_state("foo", "Running", 123, true),
            generate_process_state("bar", "Completed", 123, false),
        ]
        .into();

        let all_processes = processes_by_name_or_default_to_all(&processes, &[])
            .expect("no process names should return all processes");

        assert_eq!(all_processes.len(), 2);
    }

    #[test]
    fn processes_by_name_fails_for_invalid_names() {
        let processes = [generate_process_state("foo", "Running", 123, true)].into();
        processes_by_name_or_default_to_all(&processes, &["bar".to_string()])
            .expect_err("invalid process name should error");
    }
}
