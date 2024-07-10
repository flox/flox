use std::collections::BTreeMap;
use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use once_cell::sync::Lazy;
#[cfg(test)]
use proptest::prelude::*;
use regex_lite::Regex;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use tempfile::NamedTempFile;

use crate::flox::Flox;
use crate::models::lockfile::LockedManifestCatalog;
use crate::models::manifest::ManifestServices;
use crate::utils::traceable_path;

const PROCESS_NEVER_EXIT_NAME: &str = "flox_never_exit";
pub const SERVICES_ENV_VAR: &str = "FLOX_FEATURES_SERVICES";
pub const SERVICE_CONFIG_FILENAME: &str = "service-config.yaml";
pub static PROCESS_COMPOSE_BIN: Lazy<String> = Lazy::new(|| {
    env::var("PROCESS_COMPOSE_BIN").unwrap_or(env!("PROCESS_COMPOSE_BIN").to_string())
});

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("failed to generate service config")]
    GenerateConfig(#[source] serde_yaml::Error),
    #[error("failed to write service config")]
    WriteConfig(#[source] std::io::Error),
    #[error("services are not enabled")]
    FeatureFlagDisabled,
    #[error("services have not been started in this activation")]
    NotInActivation,
    #[error("there was a problem calling the service manager")]
    ProcessComposeCmd(#[source] std::io::Error),
    #[error(transparent)]
    LoggedError(#[from] LoggedError),
}

/// The deserialized representation of a `process-compose` config file.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub struct ProcessComposeConfig {
    #[cfg_attr(
        test,
        proptest(
            strategy = "proptest::collection::btree_map(any::<String>(), any::<ProcessConfig>(), 0..=3)"
        )
    )]
    pub processes: BTreeMap<String, ProcessConfig>,
}

/// The config for a single service
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub struct ProcessConfig {
    pub command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, proptest(strategy = "arbitrary_process_config_environment()"))]
    pub vars: Option<BTreeMap<String, String>>,
}

#[cfg(test)]
fn arbitrary_process_config_environment(
) -> impl proptest::strategy::Strategy<Value = Option<BTreeMap<String, String>>> {
    proptest::option::of(proptest::collection::btree_map(
        any::<String>(),
        any::<String>(),
        0..=3,
    ))
}

fn generate_never_exit_process() -> ProcessConfig {
    ProcessConfig {
        command: String::from("sleep infinity"),
        vars: None,
    }
}

impl From<ManifestServices> for ProcessComposeConfig {
    fn from(services: ManifestServices) -> Self {
        let processes = services
            .0
            .into_iter()
            .map(|(name, service)| {
                let command = service.command;
                let environment = service.vars.map(|vars| vars.0);
                (name, ProcessConfig {
                    command,
                    vars: environment,
                })
            })
            .collect();
        ProcessComposeConfig { processes }
    }
}

impl Serialize for ProcessComposeConfig {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut processes = self.processes.clone();
        // Inject an extra process to prevent `process-compose` from exiting when all services are stopped.
        processes.insert(
            PROCESS_NEVER_EXIT_NAME.to_string(),
            generate_never_exit_process(),
        );

        let mut state = serializer.serialize_struct("ProcessComposeConfig", 1)?;
        state.serialize_field("processes", &processes)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for ProcessComposeConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Inner {
            processes: BTreeMap<String, ProcessConfig>,
        }

        let mut inner = Inner::deserialize(deserializer)?;
        // Remove our extra process when reading back a config.
        inner.processes.remove(PROCESS_NEVER_EXIT_NAME);

        Ok(ProcessComposeConfig {
            processes: inner.processes,
        })
    }
}

// generate the config string
// write it out to the path
pub fn write_process_compose_config(
    config: &ProcessComposeConfig,
    path: impl AsRef<Path>,
) -> Result<(), ServiceError> {
    let contents = serde_yaml::to_string(&config).map_err(ServiceError::GenerateConfig)?;
    std::fs::write(path, contents).map_err(ServiceError::WriteConfig)?;
    Ok(())
}

/// Determines the location to write the service config file
pub fn service_config_write_location(temp_dir: impl AsRef<Path>) -> Result<PathBuf, ServiceError> {
    let file = NamedTempFile::new_in(temp_dir).map_err(ServiceError::WriteConfig)?;
    let (_, path) = file
        .keep()
        .map_err(|e| ServiceError::WriteConfig(e.error))?;

    Ok(path)
}

pub fn maybe_make_service_config_file(
    flox: &Flox,
    lockfile: &LockedManifestCatalog,
) -> Result<Option<PathBuf>, ServiceError> {
    let service_config_path = if flox.features.services {
        let config_path = service_config_write_location(&flox.temp_dir)?;
        write_process_compose_config(&lockfile.manifest.services.clone().into(), &config_path)?;
        tracing::debug!(path = traceable_path(&config_path), "wrote service config");
        Some(config_path)
    } else {
        None
    };
    Ok(service_config_path)
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
struct ProcessState {
    name: String,
    namespace: String,
    status: String,
    system_time: String,
    age: u64,
    is_ready: String,
    restarts: u64,
    exit_code: i32,
    pid: u64,
    #[serde(rename = "IsRunning")]
    is_running: bool,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
struct ProcessStates(Vec<ProcessState>);

impl ProcessStates {
    fn read(socket: impl AsRef<Path>) -> Result<ProcessStates, std::io::Error> {
        let mut cmd = base_process_compose_command(socket.as_ref());
        let output = cmd.arg("list").args(["--output", "json"]).output()?;
        let mut processes: ProcessStates = serde_json::from_slice(&output.stdout)?;
        processes
            .0
            .retain(|state| state.name != PROCESS_NEVER_EXIT_NAME);

        Ok(processes)
    }

    fn get_running_names(&self) -> Vec<String> {
        self.0
            .iter()
            .filter(|state| state.is_running)
            .map(|state| state.name.clone())
            .collect()
    }
}

/// Constructs a base `process-compose process` command to which additional
/// arguments can be appended.
fn base_process_compose_command(socket: impl AsRef<Path>) -> Command {
    let path = Path::new(&*PROCESS_COMPOSE_BIN);
    let mut cmd = Command::new(path);
    cmd.env("PATH", path)
        .arg("--unix-socket")
        .arg(socket.as_ref().to_string_lossy().as_ref())
        .arg("process");

    cmd
}

/// Stop service(s).
pub fn stop_services(
    socket: impl AsRef<Path>,
    names: &[impl AsRef<str>],
) -> Result<(), ServiceError> {
    let names = if names.is_empty() {
        ProcessStates::read(&socket)
            .map_err(ServiceError::ProcessComposeCmd)?
            .get_running_names()
    } else {
        names
            .iter()
            .map(|name| name.as_ref().to_string())
            .collect::<Vec<_>>()
    };

    tracing::debug!(names = names.join(","), "stopping services");

    // TODO: Better output and error handling.
    let mut cmd = base_process_compose_command(socket);
    let output = cmd
        .arg("stop")
        .args(names)
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .output()
        .map_err(ServiceError::ProcessComposeCmd)?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let logged_error: LoggedError = extract_err_msgs(&stderr)
            .ok_or(ServiceError::ProcessComposeCmd(std::io::Error::new(
                std::io::ErrorKind::Other,
                stderr.clone(),
            )))?
            .into();
        Err(ServiceError::LoggedError(logged_error))
    }
}

/// Error message extracted from process-compose logs
#[derive(Debug, Clone)]
pub struct ProcessComposeLogContents {
    pub err_msg: String,
    pub cause_msg: String,
}

/// The types of errors that are logged by process-compose
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum LoggedError {
    #[error("couldn't connect to service manager")]
    SocketDoesntExist,
    #[error("service '{0}' is not running")]
    ServiceNotRunning(String),
    #[error("unknown error: {0}")]
    Other(String),
}

/// Extracts an error message from the process-compose output if one exists.
///
/// Error messages appear in logs as:
/// <timestamp> FTL <err> error="<cause>"
fn extract_err_msgs(output: impl AsRef<str>) -> Option<ProcessComposeLogContents> {
    let output = output.as_ref();
    let err_msg_index = output.find("FTL")?;
    let cause_msg_index = output.find("error=")?;
    let len = output.len();
    let err_msg = output[err_msg_index..cause_msg_index].trim();
    let offset = 8; // 'error="' is 7 characters long
    let cause_msg = output[cause_msg_index + offset..len].trim();
    Some(ProcessComposeLogContents {
        err_msg: err_msg.to_string(),
        cause_msg: cause_msg.to_string(),
    })
}

impl From<ProcessComposeLogContents> for LoggedError {
    fn from(contents: ProcessComposeLogContents) -> Self {
        // Unwrapping is safe here, the regex is a constant
        let regex = Regex::new(r"process ([a-zA-Z0-9_-]+) is not running")
            .expect("failed to compile regex");
        if let Some(captures) = regex.captures(&contents.cause_msg) {
            LoggedError::ServiceNotRunning(
                // Unwrapping is safe here, the regex guarantees that this capture group exists
                captures
                    .get(1)
                    .expect("failed to extract capture group")
                    .as_str()
                    .to_string(),
            )
        } else if contents
            .cause_msg
            .contains("connect: no such file or directory")
        {
            LoggedError::SocketDoesntExist
        } else {
            LoggedError::Other(contents.cause_msg)
        }
    }
}

#[cfg(test)]
mod tests {
    use indoc::indoc;
    use proptest::prelude::*;
    use tempfile::TempDir;

    use super::*;

    proptest! {
        #[test]
        fn test_process_compose_config_roundtrip(config: ProcessComposeConfig) {
            let temp_dir = TempDir::new().unwrap();
            let path = service_config_write_location(&temp_dir).unwrap();
            write_process_compose_config(&config, &path).unwrap();
            let contents = std::fs::read_to_string(&path).unwrap();
            let deserialized: ProcessComposeConfig = serde_yaml::from_str(&contents).unwrap();
            prop_assert_eq!(config, deserialized);
        }
    }

    #[test]
    fn test_process_compose_config_injects_never_sleep_process() {
        // This is complimentary to the round-trip test above which doesn't see the injected process.
        let config_in = ProcessComposeConfig {
            processes: BTreeMap::from([("foo".to_string(), ProcessConfig {
                command: String::from("bar"),
                vars: None,
            })]),
        };
        let config_out = serde_yaml::to_string(&config_in).unwrap();
        assert_eq!(config_out, indoc! { "
            processes:
              flox_never_exit:
                command: sleep infinity
              foo:
                command: bar
        "})
    }
}
