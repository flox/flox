//! Service management for Flox.
//!
//! We use `process-compose` as a backend to manage services.
//!
//! Note that `process-compose` terminates when all services are stopped. To prevent this, we inject
//! a dummy service (`flox_never_exit`) that sleeps indefinitely.

use std::collections::BTreeMap;
use std::env;
use std::io::{BufRead, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, Sender};

use once_cell::sync::Lazy;
#[cfg(test)]
use proptest::prelude::*;
use regex::Regex;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use tempfile::NamedTempFile;
use tracing::{debug, trace};

use crate::flox::Flox;
use crate::models::lockfile::LockedManifestCatalog;
use crate::models::manifest::ManifestServices;
use crate::utils::{traceable_path, CommandExt};

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
    /// This variant is specifically for errors that are logged by process-compose as opposed to
    /// errors that may be encountered calling process-compose or interpreting its output.
    #[error(transparent)]
    LoggedError(#[from] LoggedError),
    #[error("failed to parse service manager output")]
    ParseOutput(#[source] serde_json::Error),
    #[error("environment doesn't have any running services")]
    NoRunningServices,
    #[error("failed to read process log line")]
    ReadLogLine(#[source] std::io::Error),
}

impl ServiceError {
    /// Constructs a `ServiceError` from the output of an unsuccessful `process-compose` command.
    pub fn from_process_compose_log(output: impl AsRef<str>) -> Self {
        extract_err_msgs(&output)
            .map(|msgs| LoggedError::from(msgs).into())
            .unwrap_or_else(|| {
                ServiceError::ProcessComposeCmd(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    output.as_ref().to_string(),
                ))
            })
    }
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

/// The parsed output of `process-compose process list` for a single process.
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
pub struct ProcessStates(Vec<ProcessState>);

impl ProcessStates {
    /// Query the status of all processes using `process-compose process list`.
    ///
    /// Note that this strips out our `flox_never_exit` process.
    pub fn read(socket: impl AsRef<Path>) -> Result<ProcessStates, ServiceError> {
        let mut cmd = base_process_compose_command(socket.as_ref());
        let output = cmd
            .arg("list")
            .args(["--output", "json"])
            .output()
            .map_err(ServiceError::ProcessComposeCmd)?;
        if !output.status.success() {
            return Err(ServiceError::from_process_compose_log(
                String::from_utf8_lossy(&output.stderr),
            ));
        }
        let mut processes: ProcessStates =
            serde_json::from_slice(&output.stdout).map_err(ServiceError::ParseOutput)?;
        processes
            .0
            .retain(|state| state.name != PROCESS_NEVER_EXIT_NAME);

        Ok(processes)
    }

    /// Get the names of processes that are currently running.
    pub fn running_process_names(&self) -> Vec<String> {
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
        .env("NO_COLOR", "1") // apparently it doesn't do this automatically even though it's not connected to a tty...
        .arg("--unix-socket")
        .arg(socket.as_ref().to_string_lossy().as_ref())
        .arg("process");

    cmd
}

/// Stop service(s) using `process-compose process stop`.
pub fn stop_services(
    socket: impl AsRef<Path>,
    names: &[impl AsRef<str>],
) -> Result<(), ServiceError> {
    let names = names.iter().map(|name| name.as_ref()).collect::<Vec<_>>();
    tracing::debug!(names = names.join(","), "stopping services");

    let mut cmd = base_process_compose_command(socket);
    let output = cmd
        .arg("stop")
        .args(names)
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .output()
        .map_err(ServiceError::ProcessComposeCmd)?;

    if output.status.success() {
        tracing::debug!("services stopped");
        Ok(())
    } else {
        tracing::debug!("stopping services failed");
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(ServiceError::from_process_compose_log(stderr))
    }
}

/// Strings extracted from a process-compose error log.
///
/// This is just raw data intended to be interpreted into a specific kind of error
/// from process-compose.
#[derive(Debug, Clone)]
pub struct ProcessComposeLogContents {
    pub err_msg: String,
    pub cause_msg: String,
}

/// The types of errors that are logged by process-compose.
///
/// These are errors formed by interpreting strings extracted from process-compose logs.
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
    // Unwrapping is safe here because the regex is a constant
    let regex = Regex::new(r#"FTL (.+) error="(.+)""#).expect("failed to compile regex");
    if let Some(captures) = regex.captures(output) {
        return Some(ProcessComposeLogContents {
            // Unwrapping is safe here, the regex guarantees that these capture groups exist
            err_msg: captures
                .get(1)
                .expect("missing first log capture group")
                .as_str()
                .to_string(),
            cause_msg: captures
                .get(2)
                .expect("missing second log capture group")
                .as_str()
                .to_string(),
        });
    }
    None
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

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ProcessComposeLogLine {
    pub process: String,
    pub message: String,
}

impl ProcessComposeLogLine {
    /// Construct a new log line.
    fn new(process: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            process: process.into(),
            message: message.into(),
        }
    }
}

pub struct ProcessComposeLogStream {
    readers: Vec<ProcessComposeLogReader>,
    rx: Receiver<ProcessComposeLogLine>,
}

impl ProcessComposeLogStream {
    /// Create a new log stream by attaching to the logs of multiple processes.
    ///
    /// For each `process` in `processes`, a new [ProcessComposeLogReader] will be started,
    /// which will read log lines for the process and send them via MPSC to the receiver.
    /// [ProcessComposeLogStream] implements [Iterator]
    /// that will wait for log lines from any of the processes.
    pub fn new(
        socket: impl AsRef<Path>,
        processes: impl IntoIterator<Item = impl AsRef<str>>,
    ) -> Result<ProcessComposeLogStream, ServiceError> {
        let (tx, rx) = std::sync::mpsc::channel();

        let readers = processes
            .into_iter()
            .map(|process| ProcessComposeLogReader::start(socket.as_ref(), process, tx.clone()))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(ProcessComposeLogStream { readers, rx })
    }
}

/// An iterator over log lines from multiple processes.
///
/// This iterator will block until a log line is received from any of the processes.
/// Once _all_ processes have stopped,
/// the iterator will return possible errors returned by the reader threads.
/// Note that as long as at least one process is running,
/// the iterator will keep outputting logs from that process.
///
/// Consider: send errors via the channel, to end logging early.
impl Iterator for ProcessComposeLogStream {
    type Item = Result<ProcessComposeLogLine, ServiceError>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.rx.recv() {
            Ok(line) => Some(Ok(line)),
            // All senders have been dropped, so we wont't receive any more messages.
            // Drain remaining reader return values.
            Err(_) => {
                loop {
                    let reader: ProcessComposeLogReader = self.readers.pop()?;
                    let joined = reader.handle.join().expect("reader thread panicked");
                    match joined {
                        // thread joined successfully
                        // we can continue to the next reader
                        Ok(()) => continue,
                        Err(e) => return Some(Err(e)),
                    }
                }
            },
        }
    }
}

/// Representation of a thread reading logs from a `process-compose process logs` process.
struct ProcessComposeLogReader {
    handle: std::thread::JoinHandle<Result<(), ServiceError>>,
}

impl ProcessComposeLogReader {
    /// Start a new log reader for a single process
    /// and listen to its output on a separate thread.
    ///
    /// For each logged line, a [ProcessComposeLogLine] is created
    /// and sent via an [std::sync::mpsc::channel].
    /// [ProcessComposeLogReader] is meant to be used in conjunction with [ProcessComposeLogStream],
    /// which holds the receiver end of the channel, receiving log lines from multiple readers.
    fn start(
        socket: impl AsRef<Path>,
        process: impl AsRef<str>,
        tx: Sender<ProcessComposeLogLine>,
    ) -> Result<ProcessComposeLogReader, ServiceError> {
        let socket = socket.as_ref().to_path_buf();
        let process = process.as_ref().to_string();

        let handle = std::thread::spawn(move || {
            let span = tracing::debug_span!("process-compose-log-reader", process = &process);
            let _guard = span.enter();

            let mut cmd = base_process_compose_command(socket);
            cmd.arg("logs")
                .arg(&process)
                .arg("--follow")
                .stderr(Stdio::piped())
                .stdout(Stdio::piped());

            debug!(cmd = cmd.display().to_string(), "attaching to logs");

            let mut child = cmd.spawn().map_err(ServiceError::ProcessComposeCmd)?;

            let stdout = child.stdout.take().expect("failed to get stdout");
            let reader = std::io::BufReader::new(stdout);

            // `process-compose process logs` will keep blocking even
            // when the process **and socket** are gone.
            // Thus reader.lines() will keep blocking indefinitely.
            // Todo: add a heartbeat to stop receiving logs and kill log reader processes
            for line in reader.lines() {
                let line = line.map_err(ServiceError::ReadLogLine)?;

                // The receiver end was dropped, so we can't send any more messages.
                // Might as well break out of the loop and kill the child process.
                let Ok(_) = tx.send(ProcessComposeLogLine::new(&process, line)) else {
                    debug!("receiver dropped, stopping log reader");
                    break;
                };
            }

            // Here, either sending to the receiver failed i.e. the receiver was dropped,
            // or the child process died.
            // If the child process ended, that's unexpected
            // and we'll try to communicate that through the channel.
            // The most likely error is that the socket doesn't exist,
            // trying to read logs for a non existent process
            // unfortunately just blocks indefinitely without any error message.

            if let Some(exit_status) = child.try_wait().map_err(ServiceError::ProcessComposeCmd)? {
                debug!(?exit_status, "child process exited");

                if !exit_status.success() {
                    let mut output = String::new();
                    child
                        .stderr
                        .take()
                        .unwrap()
                        .read_to_string(&mut output)
                        .map_err(ServiceError::ProcessComposeCmd)?;

                    trace!(output, "child process quit with error");

                    let err = ServiceError::from_process_compose_log(output);
                    Err(err)?;
                }
            } else {
                // The child process is still running, so we can kill it.
                child.kill().map_err(ServiceError::ProcessComposeCmd)?;
            }
            Ok(())
        });

        Ok(ProcessComposeLogReader { handle })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::thread;
    use std::time::Duration;

    use indoc::indoc;
    use itertools::Itertools;
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
    fn test_process_compose_config_injects_never_exit_process() {
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

    /// A test helper that starts a `process-compose` instance with a given [ProcessComposeConfig].
    /// The process is stopped when the instance is dropped or [TestProcessComposeInstance::stop]
    /// is called.
    struct TestProcessComposeInstance {
        _temp_dir: TempDir,
        socket: PathBuf,
        child: std::process::Child,
    }

    impl TestProcessComposeInstance {
        /// Start a `process-compose` instance with the given [ProcessComposeConfig].
        /// Wait for the socket to appear before returning.
        ///
        /// Panics if the socket doesn't appear after 5 tries with backoff.
        fn start(config: &ProcessComposeConfig) -> Self {
            let temp_dir = TempDir::new().unwrap();

            let config_path = temp_dir.path().join("config.yaml");
            let socket = temp_dir.path().join("S.process-compose");
            write_process_compose_config(config, &config_path).unwrap();

            let mut cmd = Command::new(&*PROCESS_COMPOSE_BIN);

            // apparently it doesn't do this automatically even though it's not connected to a tty...
            cmd.env("NO_COLOR", "1");
            cmd.arg("--unix-socket")
                .arg(&socket)
                .arg("--config")
                .arg(config_path)
                .arg("--tui=false")
                .arg("up")
                .stdout(Stdio::null())
                .stderr(Stdio::inherit());

            // Dropping the child as stopping is handled via a process-compose command.
            let child = cmd.spawn().unwrap();

            let max_tries = 5;
            for backoff in 1..max_tries {
                println!("waiting for socket to exist");
                thread::sleep(Duration::from_millis(100 * backoff));

                // For now just check if the socket exists.
                // Processes _may_ have not started yet, or the socket is unresponsive.
                // We can't really check if the process is running,
                // as it may have already exited.
                // We could chek if the socket can be connected to
                // or try to read ProcessStates, if the current approach leads to flaking tests.
                if socket.exists() {
                    break;
                }

                if backoff == max_tries {
                    panic!("socket never appeared");
                }
            }

            Self {
                _temp_dir: temp_dir,
                socket,
                child,
            }
        }

        /// Get the path to the socket.
        fn socket(&self) -> &Path {
            self.socket.as_ref()
        }

        /// Stop the `process-compose` instance.
        fn stop(self) {
            drop(self)
        }
    }

    /// Try to stop the process-compose instance by sending a SIGTERM
    /// to the process-compose process, which will stop all services.
    impl std::ops::Drop for TestProcessComposeInstance {
        fn drop(&mut self) {
            let term_result = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(self.child.id() as i32),
                nix::sys::signal::SIGTERM,
            );

            if let Err(e) = term_result {
                debug!("failed to send SIGTERM to process-compose: {:?}", e);
            }
        }
    }

    /// Test that [ProcessComposeLogReader] reads logs in order and sends them to the receiver.
    #[test]
    fn test_single_process_logs_received_in_order() {
        let instance = TestProcessComposeInstance::start(&ProcessComposeConfig {
            processes: [("foo".to_string(), ProcessConfig {
                command: String::from(
                    "i=0; while true; do i=$((i+1)); echo foo \"$((i))\"; sleep 0.1; done",
                ),
                vars: None,
            })]
            .into(),
        });

        let (tx, rx) = std::sync::mpsc::channel();
        let _ = ProcessComposeLogReader::start(instance.socket(), "foo", tx).unwrap();

        let logs = rx.iter().take(5).collect::<Vec<_>>();

        assert_eq!(logs, vec![
            ProcessComposeLogLine::new("foo", "foo 1"),
            ProcessComposeLogLine::new("foo", "foo 2"),
            ProcessComposeLogLine::new("foo", "foo 3"),
            ProcessComposeLogLine::new("foo", "foo 4"),
            ProcessComposeLogLine::new("foo", "foo 5"),
        ]);
    }

    /// Test that [ProcessComposeLogStream] reads logs from multiple processes in order
    /// and maintains the order of logs from each process.
    #[test]
    fn test_multiple_process_logs_received_in_order() {
        let instance = TestProcessComposeInstance::start(&ProcessComposeConfig {
            processes: [
                ("foo".to_string(), ProcessConfig {
                    command: "i=0; while true; do i=$((i+1)); echo \"$((i))\"; sleep 0.1; done"
                        .to_string(),
                    vars: None,
                }),
                ("bar".to_string(), ProcessConfig {
                    command: "i=0; while true; do i=$((i+1)); echo \"$((i))\"; sleep 0.1; done"
                        .to_string(),
                    vars: None,
                }),
            ]
            .into(),
        });

        let stream = ProcessComposeLogStream::new(instance.socket(), ["foo", "bar"])
            .unwrap()
            .map(|line| line.unwrap())
            .take(10);

        let groups = stream.group_by(|line| line.process.clone());
        let groups = groups.into_iter().collect::<HashMap<_, _>>();

        assert_eq!(groups.len(), 2, "expected two processes");

        for (process, lines) in groups.into_iter() {
            let lines = lines.collect::<Vec<_>>();
            let lines_sorted = lines
                .clone()
                .into_iter()
                .sorted_by_key(|line| line.message.clone())
                .collect::<Vec<_>>();
            assert_eq!(
                lines, lines_sorted,
                "{process} lines out of order: {lines:#?}"
            );
        }
    }

    /// Test that [ProcessComposeLogStream] returns an error when the socket doesn't exist.
    #[test]
    fn test_socket_gone() {
        let instance = TestProcessComposeInstance::start(&ProcessComposeConfig {
            processes: [("foo".to_string(), ProcessConfig {
                command: String::from(
                    "i=0; while true; do i=$((i+1)); echo foo \"$((i))\"; sleep 0.1; done",
                ),
                vars: None,
            })]
            .into(),
        });

        let socket = instance.socket().to_path_buf();
        instance.stop();

        let mut stream = ProcessComposeLogStream::new(socket, ["foo"]).unwrap();

        let first_message = stream.next().unwrap();
        // the only error in the stream should be that the socket doesn't exist
        assert!(
            matches!(
                first_message,
                Err(ServiceError::LoggedError(LoggedError::SocketDoesntExist))
            ),
            "expected socket error, got {:?}",
            first_message
        );

        let remaining_messages = stream.collect::<Vec<_>>();
        assert!(
            remaining_messages.is_empty(),
            "expected no more messages, got: {:?}",
            remaining_messages
        );
    }
}
