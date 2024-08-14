use anyhow::{anyhow, Result};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::{CoreEnvironmentError, Environment};
use flox_rust_sdk::models::lockfile::LockedManifest;
use flox_rust_sdk::models::manifest::TypedManifest;
use flox_rust_sdk::providers::services::{new_services_to_start, ProcessState, ProcessStates};
use tracing::instrument;

use super::{ConcreteEnvironment, EnvironmentSelect};
use crate::commands::activate::Activate;
use crate::config::Config;

mod logs;
mod restart;
mod start;
mod status;
mod stop;

#[derive(Debug, thiserror::Error)]
pub enum ServicesCommandsError {
    #[error("Services are not currently supported for remote environments.")]
    RemoteEnvsNotSupported,
    #[error(
        "Cannot {action} services for an environment that is not activated.

To activate and start services, run 'flox activate --start-services'"
    )]
    NotInActivation { action: String },
    #[error("Environment doesn't have any services defined.")]
    NoDefinedServices,
}

/// Services Commands.
#[derive(Debug, Clone, Bpaf)]
pub enum ServicesCommands {
    /// Restart a service or services
    #[bpaf(command)]
    Restart(#[bpaf(external(restart::restart))] restart::Restart),

    /// Ensure a service or services are running
    #[bpaf(command, footer("Run 'man flox-services-start' for more details."))]
    Start(#[bpaf(external(start::start))] start::Start),

    /// Status of a service or services
    #[bpaf(command, footer("Run 'man flox-services-status' for more details."))]
    Status(#[bpaf(external(status::status))] status::Status),

    /// Ensure a service or services are stopped
    #[bpaf(command, footer("Run 'man flox-services-stop' for more details."))]
    Stop(#[bpaf(external(stop::stop))] stop::Stop),

    /// Print logs of services
    #[bpaf(command, footer("Run 'man flox-services-logs' for more details."))]
    Logs(#[bpaf(external(logs::logs))] logs::Logs),
}

impl ServicesCommands {
    #[instrument(name = "services", skip_all)]
    pub async fn handle(self, config: Config, flox: Flox) -> Result<()> {
        match self {
            ServicesCommands::Restart(args) => args.handle(config, flox).await?,
            ServicesCommands::Start(args) => args.handle(config, flox).await?,
            ServicesCommands::Status(args) => args.handle(flox).await?,
            ServicesCommands::Stop(args) => args.handle(flox).await?,
            ServicesCommands::Logs(args) => args.handle(flox).await?,
        }

        Ok(())
    }
}

/// Return a ConcreteEnvironment for variants that support services.
pub fn supported_concrete_environment(
    flox: &Flox,
    environment: &EnvironmentSelect,
) -> Result<ConcreteEnvironment> {
    let concrete_environment = environment.detect_concrete_environment(flox, "Services in")?;
    if let ConcreteEnvironment::Remote(_) = concrete_environment {
        return Err(ServicesCommandsError::RemoteEnvsNotSupported.into());
    }

    let manifest = concrete_environment.dyn_environment_ref().manifest(flox)?;
    let TypedManifest::Catalog(manifest) = manifest else {
        return Err(CoreEnvironmentError::ServicesWithV0.into());
    };
    if manifest.services.is_empty() {
        return Err(ServicesCommandsError::NoDefinedServices.into());
    }

    Ok(concrete_environment)
}

/// Return an Environment for variants that support services.
pub fn supported_environment(
    flox: &Flox,
    environment: &EnvironmentSelect,
) -> Result<Box<dyn Environment>> {
    let concrete_environment = supported_concrete_environment(flox, environment)?;
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
                    .ok_or_else(|| service_does_not_exist_error(name))
            })
            .collect::<Result<Vec<_>>>()
    } else {
        tracing::debug!("No service names provided, defaulting to all services");
        Ok(Vec::from_iter(processes.iter()))
    }
}

/// Note that this must be called within an existing activation, otherwise it
/// will leave behind a process-compose since it doesn't start a watchdog.
pub async fn start_with_new_process_compose(
    config: Config,
    flox: Flox,
    environment_select: EnvironmentSelect,
    mut concrete_environment: ConcreteEnvironment,
    names: &[String],
) -> Result<Vec<String>> {
    let environment = concrete_environment.dyn_environment_ref_mut();
    let lockfile = environment.lockfile(&flox)?;
    let LockedManifest::Catalog(lockfile) = lockfile else {
        // Checks for supported environments within the commands should prevent
        // us ever getting here, but just in case.
        return Err(CoreEnvironmentError::ServicesWithV0.into());
    };
    for name in names {
        // Check any specified names against the locked manifest that we'll use
        // for starting `process-compose`. This does a similar job as
        // `processes_by_name_or_default_to_all` where we don't yet have a
        // running `process-compose` instance.
        if !lockfile.manifest.services.contains_key(name) {
            return Err(service_does_not_exist_error(name));
        }
    }
    Activate {
        environment: environment_select,
        // We currently only check for trust for remote environments,
        // but set this to false in case that changes.
        trust: false,
        print_script: false,
        start_services: true,
        run_args: vec!["true".to_string()],
    }
    .activate(
        config,
        flox,
        concrete_environment,
        true,
        &new_services_to_start(names),
    )
    .await?;
    // We don't know if the service actually started because we don't have
    // healthchecks.
    // But we do know that activate blocks until `process-compose` is running.
    let names = if names.is_empty() {
        lockfile
            .manifest
            .services
            .keys()
            .cloned()
            .collect::<Vec<_>>()
    } else {
        names.to_vec()
    };
    Ok(names)
}

/// Error to return when a service doesn't exist, either in the lockfile or the
/// current process-compose config.
pub(crate) fn service_does_not_exist_error(name: &str) -> anyhow::Error {
    anyhow!(format!("Service '{name}' not found."))
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
