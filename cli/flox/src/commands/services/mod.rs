use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::{CoreEnvironmentError, Environment};
use flox_rust_sdk::models::lockfile::LockedManifest;
use flox_rust_sdk::models::manifest::{TypedManifest, TypedManifestCatalog};
use flox_rust_sdk::providers::services::{
    new_services_to_start,
    LoggedError,
    ProcessState,
    ProcessStates,
    ServiceError,
};
use tracing::instrument;

use super::{
    activated_environments,
    ConcreteEnvironment,
    EnvironmentSelect,
    UninitializedEnvironment,
};
use crate::commands::activate::Activate;
use crate::config::Config;
use crate::utils::message;

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
    #[error(
"Service Manager unresponsive.

Retry command or delete {socket}
and restart services with 'flox activate --start-services'",
    socket = socket.display())]
    ServiceManagerQuitUnexpectedly { socket: PathBuf },

    #[error(
        "Services not started or quit unexpectedly.

To start services, run 'flox services start' in an activated environment,
or activate the environment with 'flox activate --start-services'."
    )]
    ServiceManagerQuitOrServicesNotRunning,
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

/// An augmented [ConcreteEnvironment] that has been checked for services support.
/// Constructing a [ServicesEnvironment] requires a _local_ [ConcreteEnvironment]
/// that supports services, i.e. is defined in a v1 [TypedManifestCatalog].
///
/// Constructing a [ServicesEnvironment] for a remote environment is not supported.
///
/// The [ServicesEnvironment] provides methods to guard
pub struct ServicesEnvironment {
    environment: ConcreteEnvironment,
    socket: PathBuf,
    manifest: TypedManifestCatalog,
}

impl ServicesEnvironment {
    /// Create a [ServicesEnvironment] from a [ConcreteEnvironment].
    ///
    /// Returns an error if the environment is remote or doesn't support services.
    pub fn from_concrete_environment(
        flox: &Flox,
        environment: ConcreteEnvironment,
    ) -> Result<Self> {
        if let ConcreteEnvironment::Remote(_) = environment {
            return Err(ServicesCommandsError::RemoteEnvsNotSupported.into());
        }
        let socket = environment
            .dyn_environment_ref()
            .services_socket_path(flox)?;

        let TypedManifest::Catalog(manifest) = environment.dyn_environment_ref().manifest(flox)?
        else {
            return Err(CoreEnvironmentError::ServicesWithV0.into());
        };

        let manifest = *manifest;

        Ok(Self {
            environment,
            socket,
            manifest,
        })
    }

    /// Create a [ServicesEnvironment] from an [EnvironmentSelect],
    ///
    /// Returns an error if the environment is remote or doesn't support services.
    pub fn from_environment_selection(
        flox: &Flox,
        environment: &EnvironmentSelect,
    ) -> Result<Self> {
        let concrete_environment = environment.detect_concrete_environment(flox, "Services in")?;
        Self::from_concrete_environment(flox, concrete_environment)
    }

    /// Unwrap the [ServicesEnvironment] into the underlying [ConcreteEnvironment].
    pub fn into_inner(self) -> ConcreteEnvironment {
        self.environment
    }

    /// Get the path to the service manager socket.
    ///
    /// The socket may not exist.
    /// We currently use the existence of the socket to determine whether services are running,
    /// but this may change in the future for a more robust solution.
    pub fn socket(&self) -> &Path {
        &self.socket
    }
}

/// A guard method that can be used to ensure that services commands are available.
///
/// In this case, to use service commands, we require that the service manager socket exists
/// or that there are services defined in the environment.
///
/// As described in [Self::socket] using the `socket` to determine whether services are running,
/// may not be the most robust solution, but is currently used consistently.
pub fn guard_service_commands_available(services_environment: &ServicesEnvironment) -> Result<()> {
    if !services_environment.socket.exists() && services_environment.manifest.services.is_empty() {
        return Err(ServicesCommandsError::NoDefinedServices.into());
    }

    Ok(())
}

/// A guard method that can be used to ensure that the current process is running
/// within an activation of the [ConcreteEnvironment].
///
/// This is currently required by the [start] and [restart] commands.
pub fn guard_is_within_activation(
    services_environment: &ServicesEnvironment,
    action: &str,
) -> Result<()> {
    let activated_environments = activated_environments();

    if !activated_environments.is_active(&UninitializedEnvironment::from_concrete_environment(
        &services_environment.environment,
    )?) {
        return Err(ServicesCommandsError::NotInActivation {
            action: action.to_string(),
        }
        .into());
    }
    Ok(())
}

/// Warn about manifest changes that may require services to be restarted, if
/// the Environment has a service manager running. It doesn't guarentee that the
/// service manager is working (e.g. hasn't crashed). It is the caller's
/// responsibility to determine what manifest changes would affect services.
pub fn warn_manifest_changes_for_services(flox: &Flox, env: &dyn Environment) {
    let has_service_manager = match env.services_socket_path(flox) {
        Ok(socket) => socket.exists(),
        Err(_) => false,
    };
    if has_service_manager {
        message::warning(
            "Your manifest has changes that may require running 'flox services restart'.",
        );
    }
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

/// Add more information to connection errors when talking to the service manager.
///
/// Process-Compose commands may fail with [LoggedError::SocketDoesntExist]
/// if the specific socket doesn't exist,
/// or if the service manager has quit without cleaning up the socket,
/// i.e. the socket is unresponsive.
///
/// This function adds more context to the error in the latter case.
///
/// For practical purposes, we apply this to the [ProcessStates::read] call in
/// all services commands, as that is typically the first call that can fail.
/// For following commands, we can assume that the service manager is running.
///
/// TODO: we might move this into a `processes` method on [ServicesEnvironment],
/// as that would bring error handling closer to the environment (i.e the origin of the `socket`).
pub(super) fn handle_service_connection_error(error: ServiceError, socket: &Path) -> anyhow::Error {
    let ServiceError::LoggedError(LoggedError::SocketDoesntExist) = error else {
        return error.into();
    };

    if socket.exists() {
        return ServicesCommandsError::ServiceManagerQuitUnexpectedly {
            socket: socket.to_path_buf(),
        }
        .into();
    }

    ServicesCommandsError::ServiceManagerQuitOrServicesNotRunning.into()
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
