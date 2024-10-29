use std::path::{Path, PathBuf};

use anyhow::Result;
use bpaf::Bpaf;
use flox_rust_sdk::data::System;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::Environment;
use flox_rust_sdk::models::manifest::{Manifest, ManifestServices};
use flox_rust_sdk::providers::services::{new_services_to_start, ProcessState, ProcessStates};
use tracing::{debug, instrument};

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
    #[error(
        "Cannot {action} services for an environment that is not activated.\n\
        \n\
        To activate and start services, run 'flox activate --start-services'"
    )]
    NotInActivation { action: String },
    #[error("Environment does not have any services defined.")]
    NoDefinedServices,
    #[error("Environment does not have any services defined for '{system}'.")]
    NoDefinedServicesForSystem { system: System },
    #[error("Service '{name}' does not exist.")]
    ServiceDoesNotExist { name: String },
    #[error("Service '{name}' is not available on '{system}'.")]
    ServiceNotAvailableOnSystem { name: String, system: System },
    #[error(
        "Service '{name}' was defined after services were started.\n\
        \n\
        To use the service, restart services with 'flox services restart'"
    )]
    DefinedServiceNotActive { name: String },
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
    manifest: Manifest,
}

impl ServicesEnvironment {
    /// Create a [ServicesEnvironment] from a [ConcreteEnvironment].
    ///
    /// Returns an error if the environment doesn't support services.
    pub fn from_concrete_environment(
        flox: &Flox,
        environment: ConcreteEnvironment,
    ) -> Result<Self> {
        let socket = environment
            .dyn_environment_ref()
            .services_socket_path(flox)?;

        let manifest = environment.dyn_environment_ref().manifest(flox)?;

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

    /// Check if services are running, or can at least be expected to be running.
    /// This is currently determined by the existence of the service manager socket.
    fn expect_services_running(&self) -> bool {
        ProcessStates::read(self.socket()).is_ok()
    }
}

/// A guard method that can be used to ensure that services commands are available.
///
/// In this case, to use service commands, we require that the service manager socket exists
/// or that there are services (compatible with the current system) defined in the environment.
///
/// As described in [Self::socket] using the `socket` to determine whether services are running,
/// may not be the most robust solution, but is currently used consistently.
pub fn guard_service_commands_available(
    services_environment: &ServicesEnvironment,
    system: &System,
) -> Result<()> {
    if !services_environment.socket.exists() && services_environment.manifest.services.is_empty() {
        return Err(ServicesCommandsError::NoDefinedServices.into());
    } else if !services_environment.socket.exists()
        && services_environment
            .manifest
            .services
            .copy_for_system(system)
            .is_empty()
    {
        return Err(ServicesCommandsError::NoDefinedServicesForSystem {
            system: system.clone(),
        }
        .into());
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
/// or default to all `processes`.
/// Typically `processes` will be the result of reading the processes
/// of the currently active process-compose instance.
///
/// If names are provided, all names must be names of actual services.
/// If an invalid name is provided, an error is returned.
fn processes_by_name_or_default_to_all<'a>(
    processes: &'a ProcessStates,
    manifest_services: &ManifestServices,
    system: impl Into<System>,
    names: &[String],
) -> Result<Vec<&'a ProcessState>> {
    if names.is_empty() {
        debug!(processes = ?processes, "No service names provided, defaulting to all services");
        return Ok(Vec::from_iter(processes.iter()));
    }

    let system = &system.into();

    let services_for_system = manifest_services.copy_for_system(system);

    let mut states = Vec::with_capacity(names.len());
    for name in names {
        if let Some(state) = processes.process(name) {
            states.push(state);
            continue;
        }

        // Check if the service is available at all
        let is_defined = manifest_services.contains_key(name);
        let is_defined_for_system = services_for_system.contains_key(name);

        if !is_defined {
            Err(service_does_not_exist_error(name))?;
        }

        if !is_defined_for_system {
            Err(service_not_available_on_system_error(name, system))?;
        }

        Err(defined_service_not_active_error(name))?;
    }

    Ok(states)
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
    let system = flox.system.clone();

    for name in names {
        // Check any specified names against the locked manifest that we'll use
        // for starting `process-compose`. This does a similar job as
        // `processes_by_name_or_default_to_all` where we don't yet have a
        // running `process-compose` instance.
        if !lockfile.manifest.services.contains_key(name) {
            return Err(service_does_not_exist_error(name))?;
        }
        if !lockfile
            .manifest
            .services
            .copy_for_system(&system)
            .contains_key(name)
        {
            return Err(service_not_available_on_system_error(name, &system))?;
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
            .copy_for_system(&system)
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
pub(crate) fn service_does_not_exist_error(name: &str) -> ServicesCommandsError {
    ServicesCommandsError::ServiceDoesNotExist {
        name: name.to_string(),
    }
}

/// Error to return when a service doesn't exist, either in the lockfile or the
/// current process-compose config.
fn service_not_available_on_system_error(name: &str, system: &System) -> ServicesCommandsError {
    ServicesCommandsError::ServiceNotAvailableOnSystem {
        name: name.to_string(),
        system: system.clone(),
    }
}

/// Error to return when a service is defined in the manifest
/// but not in the current process-compose config.
fn defined_service_not_active_error(name: &str) -> ServicesCommandsError {
    ServicesCommandsError::DefinedServiceNotActive {
        name: name.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use flox_rust_sdk::models::manifest::ManifestServiceDescriptor;
    use flox_rust_sdk::providers::services::test_helpers::generate_process_state;

    use super::*;

    #[test]
    fn processes_by_name_returns_named_processes() {
        let processes = [
            generate_process_state("foo", "Running", 123, true),
            generate_process_state("bar", "Completed", 123, false),
        ]
        .into();

        let all_processes = processes_by_name_or_default_to_all(
            &processes,
            &ManifestServices::default(),
            "ignore-system",
            &["foo".to_string()],
        )
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

        let all_processes = processes_by_name_or_default_to_all(
            &processes,
            &ManifestServices::default(),
            "ignore-system",
            &[],
        )
        .expect("no process names should return all processes");

        assert_eq!(all_processes.len(), 2);
    }

    #[test]
    fn processes_by_name_fails_for_invalid_names() {
        let processes = [generate_process_state("foo", "Running", 123, true)].into();
        processes_by_name_or_default_to_all(
            &processes,
            &ManifestServices::default(),
            "ignore-system",
            &["bar".to_string()],
        )
        .expect_err("invalid process name should error");
    }

    #[test]
    fn processes_by_name_fails_if_service_not_available_on_current_system() {
        let processes = [].into();
        let mut manifest_services = ManifestServices::default();
        manifest_services.insert("foo".to_string(), ManifestServiceDescriptor {
            command: "".to_string(),
            vars: None,
            is_daemon: None,
            shutdown: None,
            systems: Some(vec!["another-system".to_string()]),
        });

        let err: ServicesCommandsError = processes_by_name_or_default_to_all(
            &processes,
            &manifest_services,
            "ignore-system",
            &["foo".to_string()],
        )
        .expect_err("invalid system should error")
        .downcast()
        .unwrap();

        let expected = ServicesCommandsError::ServiceNotAvailableOnSystem {
            name: "foo".to_string(),
            system: "ignore-system".into(),
        };

        assert_eq!(
            err.to_string(),
            expected.to_string(),
            "{err:?} != {expected:?}"
        );
    }

    #[test]
    fn processes_by_name_fails_if_service_not_available_in_current_activation() {
        let processes = [].into();
        let mut manifest_services = ManifestServices::default();
        manifest_services.insert("foo".to_string(), ManifestServiceDescriptor {
            command: "".to_string(),
            vars: None,
            is_daemon: None,
            shutdown: None,
            systems: Some(vec!["system".to_string()]),
        });

        let err: ServicesCommandsError =
            processes_by_name_or_default_to_all(&processes, &manifest_services, "system", &[
                "foo".to_string()
            ])
            .expect_err("invalid system should error")
            .downcast()
            .unwrap();

        let expected = ServicesCommandsError::DefinedServiceNotActive {
            name: "foo".to_string(),
        };

        assert_eq!(
            err.to_string(),
            expected.to_string(),
            "{err:?} != {expected:?}"
        );
    }
}
