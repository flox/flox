use std::path::{Path, PathBuf};

use anyhow::Result;
use bpaf::Bpaf;
use flox_core::activate::context::InvocationType;
use flox_core::activate::mode::ActivateMode;
use flox_core::activations::{read_activations_json, state_json_path};
use flox_core::proc_status::is_descendant_of;
use flox_rust_sdk::data::System;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::Environment;
use flox_rust_sdk::models::environment::generations::GenerationId;
use flox_rust_sdk::models::lockfile::Lockfile;
use flox_rust_sdk::models::manifest::typed::{Inner, Manifest, Services};
use flox_rust_sdk::providers::services::process_compose::{ProcessState, ProcessStates};
use tracing::{debug, instrument};

use super::{
    ConcreteEnvironment,
    EnvironmentSelect,
    UninitializedEnvironment,
    activated_environments,
};
use crate::commands::activate::{Activate, CommandSelect};
use crate::commands::display_help;
use crate::config::Config;
use crate::utils::message;

mod logs;
mod persist;
mod restart;
mod start;
mod status;
mod stop;

/// The state of process-compose relative to the current activation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessComposeState {
    /// The same activation is still starting (hook.on-activate is running).
    /// Cannot start a new process-compose instance - would deadlock on ourselves.
    ActivationStartingSelf,
    /// Process-compose is running with the current store path.
    Current,
    /// Process-compose is not running or has a different store path.
    NotCurrent,
}

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
    #[error(
        "Cannot start services from 'hook.on-activate'.\n\
        \n\
        Starting services from the activation hook would cause a deadlock.\n\
        Activate the environment with 'flox activate --start-services' instead."
    )]
    CalledFromActivationHook,
}

/// Services Commands.
#[derive(Debug, Clone, Bpaf)]
pub enum ServicesCommands {
    /// Prints help information
    #[bpaf(command, hide)]
    Help,
    /// Restart a service or services
    #[bpaf(command, short('r'))]
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
    #[bpaf(
        command,
        short('l'),
        footer("Run 'man flox-services-logs' for more details.")
    )]
    Logs(#[bpaf(external(logs::logs))] logs::Logs),

    /// Generate configs for persistent system managed services
    #[bpaf(command, hide)]
    Persist(#[bpaf(external(persist::persist))] persist::Persist),
}

impl ServicesCommands {
    #[instrument(name = "services", skip_all)]
    pub async fn handle(self, config: Config, flox: Flox) -> Result<()> {
        match self {
            ServicesCommands::Help => {
                display_help(Some("services".to_string()));
            },
            ServicesCommands::Restart(args) => args.handle(config, flox).await?,
            ServicesCommands::Start(args) => args.handle(config, flox).await?,
            ServicesCommands::Status(args) => args.handle(flox).await?,
            ServicesCommands::Stop(args) => args.handle(flox).await?,
            ServicesCommands::Logs(args) => args.handle(flox).await?,
            ServicesCommands::Persist(args) => args.handle(flox).await?,
        }

        Ok(())
    }
}

/// An augmented [ConcreteEnvironment] that has been checked for services support.
/// Constructing a [ServicesEnvironment] requires a _local_ [ConcreteEnvironment]
/// that supports services, i.e. is defined in a v1 [TypedManifestCatalog].
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
    /// Will lock the environment if it's not already locked.
    ///
    /// Returns an error if the environment doesn't support services.
    pub fn from_concrete_environment(
        flox: &Flox,
        mut environment: ConcreteEnvironment,
    ) -> Result<Self> {
        let socket = environment.services_socket_path(flox)?;
        let lockfile: Lockfile = environment.lockfile(flox)?.into();
        let manifest = lockfile.manifest;

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

    /// Check the state of process-compose relative to the current activation.
    pub fn process_compose_state(
        &mut self,
        flox: &Flox,
        mode: &ActivateMode,
    ) -> ProcessComposeState {
        let state_path = state_json_path(&flox.runtime_dir, self.environment.dot_flox_path());
        let Ok((Some(state), lock)) = read_activations_json(&state_path) else {
            return ProcessComposeState::NotCurrent;
        };
        drop(lock);

        let Ok(rendered_env_links) = self.environment.rendered_env_links(flox) else {
            return ProcessComposeState::NotCurrent;
        };

        let rendered_link = rendered_env_links.for_mode(mode);
        let link_path: &Path = rendered_link.as_ref();
        let Ok(current_store_path) = std::fs::read_link(link_path) else {
            return ProcessComposeState::NotCurrent;
        };

        // Check if activation is still starting (hook.on-activate running)
        if let Some((starting_pid, starting_store_path)) = state.starting_pid_and_store_path()
            && starting_store_path == current_store_path
            && is_descendant_of(starting_pid)
        {
            // We're a descendant of the starting process with the same store path.
            // This means we're calling from within the activation hooks which would deadlock.
            return ProcessComposeState::ActivationStartingSelf;
        }

        if !self.socket.exists() {
            return ProcessComposeState::NotCurrent;
        }

        if state.process_compose_is_current(Some(&current_store_path)) {
            ProcessComposeState::Current
        } else {
            ProcessComposeState::NotCurrent
        }
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
    if !services_environment.socket.exists()
        && services_environment.manifest.services.inner().is_empty()
    {
        return Err(ServicesCommandsError::NoDefinedServices.into());
    } else if !services_environment.socket.exists()
        && services_environment
            .manifest
            .services
            .copy_for_system(system)
            .inner()
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
) -> Result<(ActivateMode, Option<GenerationId>)> {
    let activated_environments = activated_environments();

    let env =
        UninitializedEnvironment::from_concrete_environment(&services_environment.environment);

    if let Some(active) = activated_environments.get_if_active(&env) {
        Ok((active.mode.clone(), active.generation))
    } else {
        Err(ServicesCommandsError::NotInActivation {
            action: action.to_string(),
        }
        .into())
    }
}

/// Warn about manifest changes that may require services to be restarted, if
/// the Environment has a service manager running. It doesn't guarantee that the
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
    manifest_services: &Services,
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
        let is_defined = manifest_services.inner().contains_key(name);
        let is_defined_for_system = services_for_system.inner().contains_key(name);

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

/// Run an ephemeral activation to start services with a new process-compose.
///
/// This is used by `services start` and `services restart` when a new
/// process-compose instance is needed.
pub async fn start_services_with_new_process_compose(
    config: Config,
    flox: Flox,
    environment_select: EnvironmentSelect,
    mut concrete_environment: ConcreteEnvironment,
    activate_mode: ActivateMode,
    names: &[String],
    generation: Option<GenerationId>,
) -> Result<Vec<String>> {
    let lockfile: Lockfile = concrete_environment.lockfile(&flox)?.into();
    let system = flox.system.clone();

    let names: Vec<String> = if names.is_empty() {
        lockfile
            .manifest
            .services
            .copy_for_system(&system)
            .inner()
            .keys()
            .cloned()
            .collect()
    } else {
        // Check any specified names against the locked manifest that we'll use
        // for starting `process-compose`. This does a similar job as
        // `processes_by_name_or_default_to_all` where we don't yet have a
        // running `process-compose` instance.
        let all_services = lockfile.manifest.services.inner();
        let system_services = lockfile.manifest.services.copy_for_system(&system);
        let system_services = system_services.inner();

        for name in names {
            if !all_services.contains_key(name) {
                return Err(service_does_not_exist_error(name))?;
            }
            if !system_services.contains_key(name) {
                return Err(service_not_available_on_system_error(name, &system))?;
            }
        }
        names.to_vec()
    };

    Activate {
        environment: environment_select,
        // We currently only check for trust for remote environments,
        // but set this to false in case that changes.
        trust: false,
        print_script: false,
        start_services: true,
        mode: Some(activate_mode),
        generation,
        // this isn't actually used because we pass invocation type below
        command: Some(CommandSelect::ExecCommand {
            exec_command: vec!["true".to_string()],
        }),
    }
    .activate(
        config,
        flox,
        concrete_environment,
        InvocationType::ExecCommand(vec!["true".to_string()]),
        names.to_vec(),
    )
    .await?;
    // We don't know if the service actually started because we don't have
    // healthchecks.
    // But we do know that activate blocks until `process-compose` is running.
    Ok(names)
}

/// Error to return when a service doesn't exist, either in the lockfile or the
/// current process-compose config.
pub(crate) fn service_does_not_exist_error(name: &str) -> ServicesCommandsError {
    ServicesCommandsError::ServiceDoesNotExist {
        name: name.to_string(),
    }
}

/// Error to return when a service doesn't exist for the current system,
/// either in the lockfile or the current process-compose config.
pub(crate) fn service_not_available_on_system_error(
    name: &str,
    system: &System,
) -> ServicesCommandsError {
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
    use flox_rust_sdk::models::manifest::typed::ServiceDescriptor;
    use flox_rust_sdk::providers::services::process_compose::test_helpers::generate_process_state;

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
            &Services::default(),
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
            &Services::default(),
            "ignore-system",
            &[],
        )
        .expect("no process names should return all processes");

        assert_eq!(all_processes.len(), 2);
    }

    #[test]
    fn processes_by_name_fails_for_invalid_names() {
        let processes = [generate_process_state("foo", "Running", 123, true)].into();
        processes_by_name_or_default_to_all(&processes, &Services::default(), "ignore-system", &[
            "bar".to_string(),
        ])
        .expect_err("invalid process name should error");
    }

    #[test]
    fn processes_by_name_fails_if_service_not_available_on_current_system() {
        let processes = [].into();
        let mut manifest_services = Services::default();
        manifest_services
            .inner_mut()
            .insert("foo".to_string(), ServiceDescriptor {
                command: "".to_string(),
                vars: None,
                is_daemon: None,
                shutdown: None,
                systemd: None,
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
        let mut manifest_services = Services::default();
        manifest_services
            .inner_mut()
            .insert("foo".to_string(), ServiceDescriptor {
                command: "".to_string(),
                vars: None,
                is_daemon: None,
                shutdown: None,
                systemd: None,
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
