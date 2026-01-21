use std::collections::BTreeMap;
use std::ops::Deref;
use std::path::{Path, PathBuf};

use anyhow::Context;
use fslock::LockFile;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::OffsetDateTime;
use tracing::debug;

use crate::activate::mode::ActivateMode;
use crate::proc_status::pid_is_running;
use crate::{Version, path_hash};

const EXECUTIVE_NOT_STARTED: Pid = 0;

type Error = anyhow::Error;
type Pid = i32;

/// Represents running processes attached to an activation.
/// Attachments take precedence over executive in this representation.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum RunningProcesses {
    /// One or more shell processes are attached to the activation
    Attachments(Vec<Pid>),
    /// No attachments, but the executive process is running
    Executive(Pid),
}

impl RunningProcesses {
    /// Construct a RunningProcesses enum from separate PID lists.
    /// Filters to running PIDs and applies precedence (attachments > executive).
    fn from_pids(attached_pids: Vec<Pid>, executive_pid: Pid) -> Option<Self> {
        let running_attached: Vec<Pid> = attached_pids
            .into_iter()
            .filter(|pid| pid_is_running(*pid))
            .collect();

        let running_executive = Some(executive_pid)
            .filter(|&pid| pid != EXECUTIVE_NOT_STARTED)
            .filter(|&pid| pid_is_running(pid));

        if !running_attached.is_empty() {
            Some(RunningProcesses::Attachments(running_attached))
        } else {
            running_executive.map(RunningProcesses::Executive)
        }
    }
}

#[derive(Debug, Eq, PartialEq, thiserror::Error)]
pub enum UnsupportedVersion {
    /// ActivationState of unsupported version with running activations.
    WithRunningAttachments { pids: Vec<Pid> },
    /// ActivationState of unsupported version with no running activations but a running executive.
    WithRunningExecutive { pid: Pid },
}

impl UnsupportedVersion {
    pub fn from_running_processes(running: RunningProcesses) -> Self {
        match running {
            RunningProcesses::Attachments(pids) => {
                UnsupportedVersion::WithRunningAttachments { pids }
            },
            RunningProcesses::Executive(pid) => UnsupportedVersion::WithRunningExecutive { pid },
        }
    }
}

impl std::fmt::Display for UnsupportedVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UnsupportedVersion::WithRunningAttachments { pids } => {
                let pid_list = pids
                    .iter()
                    .map(|i| i.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");

                write!(
                    f,
                    "This environment has already been activated with an incompatible version of 'flox'.\n\n\
                     Exit all activations of the environment and try again.\n\
                     PIDs of the running activations: {pid_list}",
                )
            },
            UnsupportedVersion::WithRunningExecutive { pid: executive_pid } => {
                write!(
                    f,
                    "This environment has already been activated with an incompatible version of 'flox'.\n\n\
                     The executive process is still running.\n\
                     Wait for it to finish, or stop it with: 'kill {executive_pid}'",
                )
            },
        }
    }
}

#[derive(Debug, Eq, PartialEq, thiserror::Error)]
pub enum ModeMismatch {
    /// Mode mismatch with running attachments.
    WithRunningAttachments {
        current_mode: crate::activate::mode::ActivateMode,
        requested_mode: crate::activate::mode::ActivateMode,
        pids: Vec<Pid>,
    },
    /// Mode mismatch with no running attachments but a running executive.
    WithRunningExecutive {
        current_mode: crate::activate::mode::ActivateMode,
        requested_mode: crate::activate::mode::ActivateMode,
        pid: Pid,
    },
}

impl ModeMismatch {
    pub fn from_running_processes(
        current_mode: crate::activate::mode::ActivateMode,
        requested_mode: crate::activate::mode::ActivateMode,
        running: RunningProcesses,
    ) -> Self {
        match running {
            RunningProcesses::Attachments(pids) => ModeMismatch::WithRunningAttachments {
                current_mode,
                requested_mode,
                pids,
            },
            RunningProcesses::Executive(pid) => ModeMismatch::WithRunningExecutive {
                current_mode,
                requested_mode,
                pid,
            },
        }
    }
}

impl std::fmt::Display for ModeMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModeMismatch::WithRunningAttachments {
                current_mode,
                requested_mode,
                pids,
            } => {
                let pid_list = pids
                    .iter()
                    .map(|i| i.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");

                write!(
                    f,
                    "Environment can't be activated in '{requested_mode}' mode whilst there are existing activations in '{current_mode}' mode\n\n\
                     Exit all activations of the environment and try again.\n\
                     PIDs of the running activations: {pid_list}",
                )
            },
            ModeMismatch::WithRunningExecutive {
                current_mode,
                requested_mode,
                pid,
            } => {
                write!(
                    f,
                    "Environment can't be activated in '{requested_mode}' mode whilst there are existing activations in '{current_mode}' mode\n\n\
                     The executive process is still running.\n\
                     Wait for it to finish, or stop it with: 'kill {pid}'",
                )
            },
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Deserialize, Serialize)]
pub struct AttachedPid {
    pub pid: i32,
    /// If Some, the time after which the activation can be cleaned up
    ///
    /// Even if the PID has exited, the activation should not be cleaned up
    /// until an expiration is reached.
    /// Expiration is used to support in-place activations.
    /// For an in-place activation, the `flox activate` command generating the
    /// script that can be evaluated by the shell will exit before the shell has
    /// time to evaluate the script.
    /// In that case, `flox activate` sets an expiration so that the shell has
    /// some time before the activation is cleaned up.
    pub expiration: Option<OffsetDateTime>,
}

/// Acquires the filesystem-based lock on state.json
pub fn acquire_activations_json_lock(
    activations_json_path: impl AsRef<Path>,
) -> Result<LockFile, Error> {
    let lock_path = activations_json_lock_path(activations_json_path);
    let lock_path_parent = lock_path.parent().expect("lock path has parent");
    if !(lock_path.exists()) {
        std::fs::create_dir_all(lock_path_parent)?;
    }
    let mut lock = LockFile::open(&lock_path).context("failed to open lockfile")?;
    lock.lock().context("failed to lock lockfile")?;
    Ok(lock)
}

/// Returns the path to the lock file for state.json.
/// The presence of the lock file does not indicate an active lock because the
/// file isn't removed after use.
/// This is a separate file because we replace state.json on write.
fn activations_json_lock_path(activations_json_path: impl AsRef<Path>) -> PathBuf {
    activations_json_path.as_ref().with_extension("lock")
}

/// Base state directory for activations (plural) of the given environment.
///
/// `dot_flox_path` should be canonicalized before being passed to this
/// function. We can't enforce the type here because the `executive` needs to
/// still be able to read state if the environment has been deleted beneath it.
///
/// If there's a FloxHub account `activations` we'll put gcroots in this dir,
/// but it shouldn't collide with any of the hashed directories we're storing
///
/// {flox_runtime_dir}/activations/{path_hash(dot_flox_path)}-{basename(dot_flox_path)}/
pub fn activation_state_dir_path(
    runtime_dir: impl AsRef<Path>,
    dot_flox_path: impl AsRef<Path>,
) -> PathBuf {
    let dot_flox_path = dot_flox_path.as_ref();
    let hash = path_hash(dot_flox_path);
    let basename = dot_flox_path
        .parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("root");

    runtime_dir
        .as_ref()
        .join("activations")
        .join(format!("{}-{}", hash, basename))
}

/// State file for activations (plural) of the given environment.
///
/// {activation_state_dir_path}/state.json
pub fn state_json_path(runtime_dir: impl AsRef<Path>, dot_flox_path: impl AsRef<Path>) -> PathBuf {
    activation_state_dir_path(runtime_dir, dot_flox_path).join("state.json")
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StartOrAttachResult {
    /// A new activation was started for the given StartIdentifier
    Start { start_id: StartIdentifier },
    /// Attached to an existing ready activation with the given StartIdentifier
    Attach { start_id: StartIdentifier },
    /// Another process is currently starting an activation.
    /// The caller should wait and retry.
    AlreadyStarting { pid: Pid, start_id: StartIdentifier },
}

#[derive(
    Clone, Debug, Deserialize, derive_more::Display, Eq, PartialEq, Serialize, Ord, PartialOrd,
)]
pub struct UnixTimestampMillis(i64);

impl UnixTimestampMillis {
    pub fn now() -> Self {
        let now = OffsetDateTime::now_utc();
        let millis = (now.unix_timestamp_nanos() / 1_000_000) as i64;
        Self(millis)
    }
}

impl Deref for UnixTimestampMillis {
    type Target = i64;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::str::FromStr for UnixTimestampMillis {
    type Err = std::num::ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse::<i64>().map(UnixTimestampMillis)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, Ord, PartialOrd)]
pub struct StartIdentifier {
    pub store_path: PathBuf,
    pub timestamp: UnixTimestampMillis,
}

impl StartIdentifier {
    /// Compute start state directory path for this identifier.
    ///
    /// Format: {runtime_dir}/activations/{env_hash}-{env_name}/{storepath_basename}.{unix_epoch}/
    pub fn state_dir_path(
        &self,
        runtime_dir: impl AsRef<Path>,
        dot_flox_path: impl AsRef<Path>,
    ) -> Result<PathBuf, Error> {
        let base_dir = activation_state_dir_path(runtime_dir, dot_flox_path);
        let storepath_basename = self
            .store_path
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("Invalid store path"))?
            .to_string_lossy();

        let dir_name = format!("{}.{}", storepath_basename, *self.timestamp);

        Ok(base_dir.join(dir_name))
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Attachment {
    start_id: StartIdentifier,
    expiration: Option<OffsetDateTime>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
enum Ready {
    #[default]
    False,
    True(StartIdentifier),
    Starting(Pid, StartIdentifier),
}

/// Information about the activated environment.
///
/// This is only intended for humans to debug the serialized state.
/// Fields should be promoted to the top-level if they are later needed
/// programmatically.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct EnvironmentInfo {
    /// Path to the activated environment's .flox directory
    dot_flox_path: PathBuf,
    /// Path to the activated environment's .flox/run/{symlink} which encapsulates mode and platform
    flox_env: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ActivationState {
    version: Version<3>,
    info: EnvironmentInfo,
    mode: ActivateMode,
    ready: Ready,
    /// Pid must be a non-zero value when writing state to disk.
    executive_pid: Pid,
    current_process_compose_store_path: Option<StartIdentifier>,
    attached_pids: BTreeMap<Pid, Attachment>,
}

impl ActivationState {
    pub fn new(
        mode: &ActivateMode,
        dot_flox_path: impl AsRef<Path>,
        flox_env: impl AsRef<Path>,
    ) -> Self {
        Self {
            version: Version,
            info: EnvironmentInfo {
                dot_flox_path: dot_flox_path.as_ref().to_path_buf(),
                flox_env: flox_env.as_ref().to_path_buf(),
            },
            mode: mode.clone(),
            ready: Ready::default(),
            executive_pid: EXECUTIVE_NOT_STARTED,
            current_process_compose_store_path: None,
            attached_pids: BTreeMap::new(),
        }
    }

    /// Returns the list of attached PIDs that are still running.
    pub fn attached_pids_running(&self) -> Vec<Pid> {
        self.attached_pids
            .keys()
            .copied()
            .filter(|pid| pid_is_running(*pid))
            .collect()
    }

    /// Returns a mapping of StartIdentifier to info for each attachment to that StartIdentifier
    pub fn attachments_by_start_id(
        &self,
    ) -> BTreeMap<StartIdentifier, Vec<(Pid, Option<OffsetDateTime>)>> {
        self.attached_pids
            .iter()
            .fold(BTreeMap::new(), |mut acc, (pid, attachment)| {
                let Attachment {
                    expiration,
                    start_id,
                } = attachment;
                acc.entry(start_id.clone())
                    .or_default()
                    .push((*pid, *expiration));
                acc
            })
    }

    pub fn attached_pids_is_empty(&self) -> bool {
        self.attached_pids.is_empty()
    }

    /// Returns the current activation mode
    pub fn mode(&self) -> &ActivateMode {
        &self.mode
    }

    /// Check if the activation state has running processes.
    pub fn running_processes(&self) -> Option<RunningProcesses> {
        let attached_pids: Vec<Pid> = self.attached_pids.keys().copied().collect();
        RunningProcesses::from_pids(attached_pids, self.executive_pid)
    }

    /// Start or attach to an activation for the given store path.
    pub fn start_or_attach(
        &mut self,
        pid: Pid,
        store_path: impl AsRef<Path>,
    ) -> StartOrAttachResult {
        if let Ready::Starting(starting_pid, ref start_id) = self.ready
            && pid_is_running(starting_pid)
        {
            return StartOrAttachResult::AlreadyStarting {
                pid: starting_pid,
                start_id: start_id.clone(),
            };
        }

        let ready = self.ready.clone();
        match ready {
            Ready::True(start_id) if start_id.store_path == store_path.as_ref() => {
                self.attach(pid, Attachment {
                    start_id: start_id.clone(),
                    expiration: None,
                });
                StartOrAttachResult::Attach { start_id }
            },
            Ready::False | Ready::True(_) | Ready::Starting(_, _) => {
                let start_id = self.start(pid, &store_path);
                StartOrAttachResult::Start { start_id }
            },
        }
    }

    /// Set the executive PID after spawning the executive process
    pub fn set_executive_pid(&mut self, pid: Pid) {
        debug!(pid, "setting executive PID");
        self.executive_pid = pid;
    }

    /// Mark an activation as ready after hooks have completed
    pub fn set_ready(&mut self, start_id: &StartIdentifier) {
        debug!(?start_id, "marking activation as ready");
        self.ready = Ready::True(start_id.clone());
    }

    /// Detach a PID from an activation
    ///
    /// update_ready_after_detach must be called after calling detach
    pub fn detach(&mut self, pid: Pid) {
        let removed = self.attached_pids.remove(&pid);
        debug!(pid, ?removed, "detaching from activation");
    }

    /// Clean up terminated PIDs
    ///
    /// Returns a list of start IDs that have no more attached PIDs, and a boolean
    /// indicating if any PIDs were detached.
    pub fn cleanup_pids(
        &mut self,
        pid_is_running: impl Fn(Pid) -> bool,
        now: OffsetDateTime,
    ) -> (Vec<StartIdentifier>, bool) {
        let mut modified = false;
        let attachments_by_start_id = self.attachments_by_start_id();
        let mut empty_start_ids = Vec::new();

        for (start_id, attachments) in attachments_by_start_id {
            let mut all_pids_terminated = true;
            for (pid, expiration) in attachments {
                let keep_attachment = if let Some(expiration) = expiration {
                    // If the PID has an unreached expiration, retain it even if it
                    // isn't running
                    now < expiration || pid_is_running(pid)
                } else {
                    pid_is_running(pid)
                };

                if keep_attachment {
                    // We can skip checking other PIDs for this start_id because
                    // it still has attachments.
                    all_pids_terminated = false;
                    break;
                } else {
                    tracing::info!(?pid, ?start_id, "detaching terminated PID");
                    self.detach(pid);
                    modified = true;
                }
            }

            if all_pids_terminated {
                empty_start_ids.push(start_id);
            }
        }
        // Only update ready state if there are still attached PIDs
        if !self.attached_pids.is_empty() {
            self.update_ready_after_detach();
        }
        (empty_start_ids, modified)
    }

    /// set ready to False if there are no more PIDs attached to the current start
    /// should only be called when there are some attached PIDs
    fn update_ready_after_detach(&mut self) {
        if self.attached_pids.is_empty() {
            unreachable!("should remove all state when there are no more attached PIDs");
        }
        match self.ready {
            Ready::True(ref start_id) => {
                if !self.attachments_by_start_id().contains_key(start_id) {
                    debug!(?start_id, "no more attached PIDs, marking as not ready");
                    self.ready = Ready::False;
                }
            },
            // we'll let the starting process (or a subsequent start or
            // attach) handle cleanup for a dead Starting PID
            Ready::Starting(_, _) => {},
            Ready::False => {}, // no-op
        }
    }

    fn start(&mut self, pid: Pid, store_path: impl AsRef<Path>) -> StartIdentifier {
        let start_id = StartIdentifier {
            store_path: store_path.as_ref().to_path_buf(),
            timestamp: UnixTimestampMillis::now(),
        };
        let attachment = Attachment {
            start_id: start_id.clone(),
            expiration: None,
        };

        debug!(pid, ?start_id, "starting new activation");
        self.ready = Ready::Starting(pid, start_id.clone());
        self.attached_pids.insert(pid, attachment);
        start_id
    }

    fn attach(&mut self, pid: Pid, attachment: Attachment) {
        let replaced = self.attached_pids.insert(pid, attachment.clone());
        debug!(
            pid,
            ?attachment,
            ?replaced,
            "attaching to an existing activation"
        );
    }

    /// Check if executive has been started.
    pub fn executive_started(&self) -> bool {
        self.executive_pid != EXECUTIVE_NOT_STARTED
    }

    /// Check if executive was started and is running.
    pub fn executive_running(&self) -> bool {
        self.executive_started() && pid_is_running(self.executive_pid)
    }

    pub fn replace_attachment(
        &mut self,
        start_id: StartIdentifier,
        old_pid: Pid,
        new_pid: Pid,
        expiration: Option<OffsetDateTime>,
    ) -> Result<(), Error> {
        let old_attachment = self.attached_pids.remove(&old_pid).ok_or(anyhow::anyhow!(
            "PID {} not attached to activation",
            old_pid
        ))?;

        if old_attachment.start_id != start_id {
            anyhow::bail!("PID {} is not attached to the expected start", old_pid);
        }

        let new_attachment = Attachment {
            start_id,
            expiration,
        };

        self.attached_pids.insert(new_pid, new_attachment);
        Ok(())
    }
}

/// Best-effort extraction of running PIDs from unknown version JSON.
/// Returns RunningProcesses if any processes are still running.
fn extract_running_pids_from_json(content: &str) -> Result<Option<RunningProcesses>, Error> {
    #[derive(Debug, Deserialize)]
    struct PidExtractor {
        #[serde(default)]
        executive_pid: Pid,
        #[serde(default)]
        attached_pids: BTreeMap<Pid, Value>,
    }

    let extractor: PidExtractor =
        serde_json::from_str(content).context("Failed to extract PIDs from state.json")?;

    let attached_pids: Vec<Pid> = extractor.attached_pids.keys().copied().collect();

    Ok(RunningProcesses::from_pids(
        attached_pids,
        extractor.executive_pid,
    ))
}

/// Parse activation state with version checking.
/// Returns None if state should be discarded (different version and no running PIDs).
fn parse_versioned_activation_state(content: &str) -> Result<Option<ActivationState>, Error> {
    #[derive(Debug, Deserialize)]
    struct VersionOnly {
        version: Value,
    }

    let version_check: VersionOnly =
        serde_json::from_str(content).context("Failed to parse state.json")?;

    match version_check.version.as_u64() {
        // Current version.
        Some(3) => {
            let state: ActivationState =
                serde_json::from_str(content).context("Failed to parse state.json")?;
            Ok(Some(state))
        },
        // Versions 1 and 2 were stored in a different path so we don't need to handle migrations.
        // This also handles the case where someone upgrades and then downgrades Flox.
        _ => {
            let running = extract_running_pids_from_json(content)?;

            if let Some(running) = running {
                Err(UnsupportedVersion::from_running_processes(running).into())
            } else {
                debug!(
                    "discarding state.json due to unsupported version with no running attachments or executive"
                );
                Ok(None)
            }
        },
    }
}

/// Returns the parsed `state.json` file or `None` if:
///
/// - the file does not exist
/// - the version is different but there are no running processes
///
/// The file can be written with [write_activations_json].
/// This function acquires a lock on the file,
/// which should be reused for writing, to avoid TOCTOU issues.
pub fn read_activations_json(
    path: impl AsRef<Path>,
) -> Result<(Option<ActivationState>, LockFile), Error> {
    let path = path.as_ref();
    let lock_file = acquire_activations_json_lock(path).context("failed to acquire lockfile")?;

    if !path.exists() {
        debug!("activations file not found at {}", path.to_string_lossy());
        return Ok((None, lock_file));
    }

    debug!(?path, "reading state.json");
    let contents =
        std::fs::read_to_string(path).context(format!("failed to read file {}", path.display()))?;

    let parsed = parse_versioned_activation_state(&contents)?;

    Ok((parsed, lock_file))
}
/// Writes the environment `state.json` file.
/// The file is written atomically.
/// The lock is released after the write.
///
/// This uses [flox_core::serialize_atomically] to write the file, and inherits its requirements.
/// * `path` must have a parent directory.
/// * The lock must correspond to the file being written.
pub fn write_activations_json(
    activations: &ActivationState,
    path: impl AsRef<Path>,
    lock: LockFile,
) -> Result<(), Error> {
    if activations.executive_pid == EXECUTIVE_NOT_STARTED {
        anyhow::bail!(
            "Cannot write activation state without executive PID set (path: {})",
            path.as_ref().display()
        );
    }
    crate::serialize_atomically(&json!(activations), &path, lock)?;
    Ok(())
}

#[cfg(any(test, feature = "tests"))]
pub mod test_helpers {
    use std::path::Path;

    use super::{
        ActivationState,
        acquire_activations_json_lock,
        read_activations_json,
        state_json_path,
        write_activations_json,
    };

    /// Helper to write an ActivationState to disk
    ///
    /// Takes ownership of state so we don't accidentally use it after e.g. a
    /// watcher modifies state on disk
    pub fn write_activation_state(
        runtime_dir: &Path,
        dot_flox_path: &Path,
        mut state: ActivationState,
    ) {
        if !state.executive_started() {
            state.set_executive_pid(1);
        }
        let state_json_path = state_json_path(runtime_dir, dot_flox_path);
        let lock = acquire_activations_json_lock(&state_json_path).expect("failed to acquire lock");
        write_activations_json(&state, &state_json_path, lock).expect("failed to write state");
    }

    /// Helper to read an ActivationState from disk
    pub fn read_activation_state(runtime_dir: &Path, dot_flox_path: &Path) -> ActivationState {
        let state_json_path = state_json_path(runtime_dir, dot_flox_path);
        let (state, _lock) = read_activations_json(&state_json_path).expect("failed to read state");
        state.unwrap()
    }
}

#[cfg(test)]
mod tests {
    use std::process::{self, Child, Command};
    use std::time::Duration;

    use indoc::formatdoc;
    use tempfile::TempDir;

    use super::*;
    // NOTE: these two functions are copied from flox-rust-sdk since you can't
    //       share anything behind #[cfg(test)] across crates

    /// Start a shortlived process that we can check the PID is running.
    pub fn start_process() -> Child {
        Command::new("sleep")
            .arg("2")
            .spawn()
            .expect("failed to start")
    }

    /// Stop a shortlived process that we can check the PID is not running. It's
    /// unlikely, but not impossible, that the kernel will have not re-used the
    /// PID by the time we check it.
    pub fn stop_process(mut child: Child) {
        child.kill().expect("failed to kill");
        child.wait().expect("failed to wait");
    }

    fn make_activations(ready: Ready) -> ActivationState {
        let dot_flox_path = PathBuf::from("/test/.flox");
        ActivationState {
            version: Version,
            info: EnvironmentInfo {
                flox_env: dot_flox_path.join("run/test"),
                dot_flox_path,
            },
            mode: ActivateMode::default(),
            ready,
            executive_pid: 1, // Not used, but will be running.
            current_process_compose_store_path: None,
            attached_pids: BTreeMap::new(),
        }
    }

    fn make_start_id(path: &str) -> StartIdentifier {
        StartIdentifier {
            store_path: PathBuf::from(path),
            timestamp: UnixTimestampMillis::now(),
        }
    }

    fn make_attachment(start_id: StartIdentifier) -> Attachment {
        Attachment {
            start_id,
            expiration: None,
        }
    }

    mod read_and_write_state {
        use super::*;

        #[test]
        fn read_and_write_roundtrip() {
            let temp_dir = TempDir::new().unwrap();
            let dot_flox_path = temp_dir.path().join(".flox");
            let state_path = state_json_path(temp_dir.path(), dot_flox_path);

            let write_state = make_activations(Ready::False);
            let lock = acquire_activations_json_lock(&state_path).unwrap();
            write_activations_json(&write_state, &state_path, lock).unwrap();

            let (read_state_opt, _lock) = read_activations_json(&state_path).unwrap();
            let read_state = read_state_opt.expect("state should be present");
            assert_eq!(
                write_state, read_state,
                "written and read states should match"
            );
        }

        #[test]
        fn write_without_executive_pid_fails() {
            let temp_dir = TempDir::new().unwrap();
            let dot_flox_path = temp_dir.path().join(".flox");
            let flox_env = dot_flox_path.join("run/test");
            let state_path = state_json_path(temp_dir.path(), &dot_flox_path);

            let state = ActivationState::new(&ActivateMode::default(), dot_flox_path, flox_env);
            assert_eq!(
                state.executive_pid, EXECUTIVE_NOT_STARTED,
                "executive PID should be unset"
            );

            let lock = acquire_activations_json_lock(&state_path).unwrap();
            let result = write_activations_json(&state, &state_path, lock);
            assert_eq!(
                result.unwrap_err().to_string(),
                format!(
                    "Cannot write activation state without executive PID set (path: {})",
                    state_path.display()
                ),
                "writing state without executive PID should fail"
            );
        }
    }

    mod attached_pids_getters {
        use super::*;

        #[test]
        fn test_attached_pids_running() {
            let proc_running = start_process();
            let proc_stopped = start_process();

            let mut activations =
                ActivationState::new(&ActivateMode::default(), "/test/.flox", "/test/env");
            let store_path = PathBuf::from("/nix/store/test");

            // Start activation with first PID
            let result = activations.start_or_attach(proc_running.id() as i32, &store_path);
            let start_id = match result {
                StartOrAttachResult::Start { start_id, .. } => start_id,
                _ => panic!("Expected Start"),
            };

            // Mark ready so we can attach more PIDs
            activations.set_ready(&start_id);

            // Attach second PID
            activations.start_or_attach(proc_stopped.id() as i32, &store_path);

            stop_process(proc_stopped);

            assert_eq!(
                activations.attached_pids_running(),
                vec![proc_running.id() as i32],
                "should only return attached PIDs that are running"
            );

            stop_process(proc_running);
        }

        #[test]
        fn test_attached_pids_by_start_id() {
            let mut activations =
                ActivationState::new(&ActivateMode::default(), "/test/.flox", "/test/env");
            let store_path1 = PathBuf::from("/nix/store/path1");
            let store_path2 = PathBuf::from("/nix/store/path2");

            // Start activation with first store path
            let result = activations.start_or_attach(100, &store_path1);
            let start_id1 = match result {
                StartOrAttachResult::Start { start_id, .. } => start_id,
                _ => panic!("Expected Start"),
            };

            // Mark ready so we can attach more PIDs
            activations.set_ready(&start_id1);

            // Attach second PID to same start_id
            activations.start_or_attach(200, &store_path1);

            // Start activation with second store path (creates new start_id)
            let result = activations.start_or_attach(300, &store_path2);
            let start_id2 = match result {
                StartOrAttachResult::Start { start_id, .. } => start_id,
                _ => panic!("Expected Start"),
            };

            let expected = BTreeMap::from([
                (start_id1, vec![(100, None), (200, None)]),
                (start_id2, vec![(300, None)]),
            ]);
            assert_eq!(
                activations.attachments_by_start_id(),
                expected,
                "should return attached PIDs grouped by StartIdentifier"
            );
        }
    }

    mod start_or_attach {
        use super::*;

        #[test]
        fn test_start_or_attach_starts_when_ready_false() {
            let store_path = PathBuf::from("/nix/store/path1");
            let mut activations = make_activations(Ready::False);

            let pid = 123;
            let result = activations.start_or_attach(pid, &store_path);

            let start_id = match result {
                StartOrAttachResult::Start { start_id } => start_id,
                _ => panic!("Expected StartOrAttachResult::Start, got {:?}", result),
            };

            let (pid, ready_start_id) = match &activations.ready {
                Ready::Starting(p, s) => (*p, s.clone()),
                _ => panic!("Expected Ready::Starting"),
            };
            assert_eq!(pid, pid);
            assert_eq!(start_id.store_path, store_path);
            assert_eq!(start_id, ready_start_id);
            assert_eq!(
                activations.attached_pids,
                BTreeMap::from([(pid, make_attachment(start_id))])
            );
        }

        #[test]
        fn test_start_or_attach_attaches_when_ready_true_same_path() {
            let start_id = make_start_id("/nix/store/path1");
            let mut activations = make_activations(Ready::True(start_id.clone()));

            let pid = 123;
            let result = activations.start_or_attach(pid, &start_id.store_path);

            match result {
                StartOrAttachResult::Attach { start_id: id } => {
                    assert_eq!(id, start_id);
                },
                _ => panic!("Expected StartOrAttachResult::Attach, got {:?}", result),
            }

            assert_eq!(activations.ready, Ready::True(start_id.clone()));
            assert_eq!(
                activations.attached_pids,
                BTreeMap::from([(pid, make_attachment(start_id))])
            );
        }

        #[test]
        fn test_start_or_attach_starts_when_ready_true_different_path() {
            let existing = make_start_id("/nix/store/path1");
            let new_path = PathBuf::from("/nix/store/path2");
            let mut activations = make_activations(Ready::True(existing));

            let pid = 123;
            let result = activations.start_or_attach(pid, &new_path);

            let start_id = match result {
                StartOrAttachResult::Start { start_id } => start_id,
                _ => panic!("Expected StartOrAttachResult::Start, got {:?}", result),
            };

            let (ready_pid, ready_start_id) = match &activations.ready {
                Ready::Starting(p, s) => (*p, s.clone()),
                _ => panic!("Expected Ready::Starting"),
            };
            assert_eq!(ready_pid, pid);
            assert_eq!(start_id.store_path, new_path);
            assert_eq!(start_id, ready_start_id);
            assert_eq!(
                activations.attached_pids,
                BTreeMap::from([(pid, make_attachment(start_id))])
            );
        }

        #[test]
        fn test_start_or_attach_returns_already_starting_when_process_running() {
            let proc = start_process();
            let pid = proc.id() as i32;
            let start_id = make_start_id("/nix/store/path1");
            let mut activations = make_activations(Ready::Starting(pid, start_id.clone()));

            let result = activations.start_or_attach(123, &start_id.store_path);

            match result {
                StartOrAttachResult::AlreadyStarting {
                    pid: returned_pid,
                    start_id: returned_start_id,
                } => {
                    assert_eq!(returned_pid, pid);
                    assert_eq!(returned_start_id, start_id);
                },
                _ => panic!(
                    "Expected StartOrAttachResult::AlreadyStarting, got {:?}",
                    result
                ),
            }

            assert_eq!(activations.ready, Ready::Starting(pid, start_id));
            assert_eq!(activations.attached_pids, BTreeMap::new());

            stop_process(proc);
        }

        #[test]
        fn test_start_or_attach_starts_when_starting_process_terminated() {
            let proc = start_process();
            let stopped_pid = proc.id() as i32;
            stop_process(proc);

            let old_start_id = make_start_id("/nix/store/path1");
            let mut activations =
                make_activations(Ready::Starting(stopped_pid, old_start_id.clone()));

            let pid = 123;
            let result = activations.start_or_attach(pid, &old_start_id.store_path);

            let new_start_id = match result {
                StartOrAttachResult::Start { start_id } => start_id,
                _ => panic!("Expected StartOrAttachResult::Start, got {:?}", result),
            };

            let (ready_pid, ready_start_id) = match &activations.ready {
                Ready::Starting(p, s) => (*p, s.clone()),
                _ => panic!("Expected Ready::Starting"),
            };
            assert_eq!(ready_pid, pid);
            assert_eq!(new_start_id, ready_start_id);
            assert!(new_start_id.timestamp >= old_start_id.timestamp);
            assert_eq!(
                activations.attached_pids,
                BTreeMap::from([(pid, make_attachment(new_start_id))])
            );
        }

        #[test]
        fn test_start_or_attach_multiple_attachments() {
            let start_id = make_start_id("/nix/store/path1");
            let mut activations = make_activations(Ready::True(start_id.clone()));

            for pid in [100, 200, 300].iter() {
                let result = activations.start_or_attach(*pid, &start_id.store_path);
                match result {
                    StartOrAttachResult::Attach { start_id: id } => {
                        assert_eq!(id, start_id);
                    },
                    _ => panic!(
                        "Expected StartOrAttachResult::Attach for PID {}, got {:?}",
                        pid, result
                    ),
                }
            }

            assert_eq!(
                activations.attached_pids,
                BTreeMap::from([
                    (100, make_attachment(start_id.clone())),
                    (200, make_attachment(start_id.clone())),
                    (300, make_attachment(start_id.clone())),
                ])
            );
        }

        #[test]
        fn test_start_or_attach_replaces_existing_pid() {
            let mut activations =
                ActivationState::new(&ActivateMode::default(), "/test/.flox", "/test/env");
            let store_path = PathBuf::from("/nix/store/path1");

            let pid = 123;
            let result = activations.start_or_attach(pid, &store_path);
            let start_id = match result {
                StartOrAttachResult::Start { start_id, .. } => start_id,
                _ => panic!("Expected Start"),
            };

            // Set executive PID (PID 1 is always running)
            activations.set_executive_pid(1);

            // Mark ready so we can attach
            activations.set_ready(&start_id);

            // Attach same PID again - should replace existing attachment
            let result = activations.start_or_attach(pid, &start_id.store_path);

            match result {
                StartOrAttachResult::Attach { start_id: id } => {
                    assert_eq!(id, start_id);
                },
                _ => panic!("Expected StartOrAttachResult::Attach, got {:?}", result),
            }

            assert_eq!(
                activations.attached_pids,
                BTreeMap::from([(pid, make_attachment(start_id))]),
                "should have only one attachment",
            );
        }
    }

    #[test]
    fn test_cleanup_pids_keeps_expired_but_running_pids() {
        // Create an attachment with an expiration in the past
        let mut activations =
            ActivationState::new(&ActivateMode::default(), "/test/.flox", "/test/env");
        let start_id = make_start_id("/nix/store/test");
        let pid = 0;
        let now = OffsetDateTime::now_utc();
        let expiration = now - Duration::from_secs(10);
        let attachment = Attachment {
            start_id: start_id.clone(),
            expiration: Some(expiration),
        };
        activations.attach(pid, attachment);

        // Cleanup with PID still running
        let (empty_starts, modified) = activations.cleanup_pids(|_| true, now);
        assert!(activations.attached_pids.contains_key(&pid));
        assert!(!modified);
        assert!(empty_starts.is_empty());
    }

    #[test]
    fn test_cleanup_pids_keeps_not_running_but_not_expired_pids() {
        // Create an attachment with an expiration in the future
        let mut activations =
            ActivationState::new(&ActivateMode::default(), "/test/.flox", "/test/env");
        let start_id = make_start_id("/nix/store/test");
        let pid = 0;
        let now = OffsetDateTime::now_utc();
        let expiration = now + Duration::from_secs(10);
        let attachment = Attachment {
            start_id: start_id.clone(),
            expiration: Some(expiration),
        };
        activations.attach(pid, attachment);

        // Cleanup with PID not running
        let (empty_starts, modified) = activations.cleanup_pids(|_| false, now);
        assert!(activations.attached_pids.contains_key(&pid));
        assert!(!modified);
        assert!(empty_starts.is_empty());
    }

    mod running_processes {
        use super::*;

        #[test]
        fn running_processes_none() {
            let stopped_proc = start_process();
            let stopped_pid = stopped_proc.id() as Pid;
            stop_process(stopped_proc);
            let running = RunningProcesses::from_pids(vec![stopped_pid], stopped_pid);
            assert_eq!(
                running, None,
                "should return none when no attachments or executive are running"
            );
        }

        #[test]
        fn running_processes_attachments() {
            let pid_self = process::id() as Pid;
            let running = RunningProcesses::from_pids(vec![pid_self, pid_self], pid_self);
            assert_eq!(
                running,
                Some(RunningProcesses::Attachments(vec![pid_self, pid_self])),
                "should return attachments when any are running",
            );
        }

        #[test]
        fn running_processes_executive() {
            let pid_self = process::id() as Pid;
            let stopped_proc = start_process();
            let stopped_pid = stopped_proc.id() as Pid;
            stop_process(stopped_proc);

            let running = RunningProcesses::from_pids(vec![stopped_pid], pid_self);
            assert_eq!(
                running,
                Some(RunningProcesses::Executive(pid_self)),
                "should return executive when no attachments are running and executive is"
            );
        }
    }

    mod version_handling {
        use super::*;

        // Technically we'd never encounter this exact Version because we
        // changed the path of the state file during the 2025-12/2026-01
        // activation rewrite.
        const OLD_VERSION: Version<2> = Version;

        #[test]
        fn parse_versioned_activation_state_roundtrip() {
            let start_id = make_start_id("/nix/store/path");
            let mut state = make_activations(Ready::True(start_id.clone()));
            state.attached_pids = BTreeMap::from([(123, make_attachment(start_id))]);
            let json = serde_json::to_string(&state).unwrap();

            let parsed = parse_versioned_activation_state(&json).unwrap();
            assert!(parsed.is_some());
            assert_eq!(parsed.unwrap().version, Version);
        }

        #[test]
        fn parse_versioned_activation_state_malformed() {
            let json = "{not valid json}";

            let err = parse_versioned_activation_state(json).unwrap_err();
            assert_eq!(err.to_string(), "Failed to parse state.json");
        }

        #[test]
        fn parse_versioned_activation_state_different_version_incompatible_structure() {
            let json = json!({
                "version": OLD_VERSION,
                "mode": "dev",
                "ready": false,
                "executive_pid": EXECUTIVE_NOT_STARTED,
                "attached_pids": [123, 456], // hypothetical change of structure
            })
            .to_string();

            let err = parse_versioned_activation_state(&json).unwrap_err();
            assert_eq!(err.to_string(), "Failed to extract PIDs from state.json",);
        }

        #[test]
        fn parse_versioned_activation_state_different_version_pids_not_running() {
            let proc_stopped = start_process();
            let pid_stopped = proc_stopped.id().to_string();
            stop_process(proc_stopped);

            let json = json!({
                "version": OLD_VERSION,
                "mode": "dev",
                "ready": false,
                "executive_pid": EXECUTIVE_NOT_STARTED,
                "attached_pids": {
                    pid_stopped.to_string(): {},
                }
            })
            .to_string();

            let result = parse_versioned_activation_state(&json).unwrap();
            assert_eq!(result, None, "should discard existing state");
        }

        #[test]
        fn parse_versioned_activation_state_different_version_pids_running() {
            let proc1 = start_process();
            let proc2 = start_process();
            let pid1 = proc1.id() as i32;
            let pid2 = proc2.id() as i32;

            let json = json!({
                "version": OLD_VERSION,
                "mode": "dev",
                "ready": false,
                "executive_pid": EXECUTIVE_NOT_STARTED,
                "attached_pids": {
                    pid1.to_string(): {},
                    pid2.to_string(): {},
                },
            })
            .to_string();

            let err = parse_versioned_activation_state(&json).unwrap_err();
            let expected_msg = formatdoc! {"
                This environment has already been activated with an incompatible version of 'flox'.

                Exit all activations of the environment and try again.
                PIDs of the running activations: {pid1}, {pid2}",
            };
            assert_eq!(err.to_string(), expected_msg);

            stop_process(proc1);
            stop_process(proc2);
        }

        #[test]
        fn parse_versioned_activation_state_different_version_only_executive() {
            let exec_proc = start_process();
            let exec_pid = exec_proc.id() as i32;

            let json = json!({
                "version": OLD_VERSION,
                "mode": "dev",
                "ready": false,
                "executive_pid": exec_pid,
                "attached_pids": {},
            })
            .to_string();

            let err = parse_versioned_activation_state(&json).unwrap_err();
            let expected_msg = formatdoc! {"
                This environment has already been activated with an incompatible version of 'flox'.

                The executive process is still running.
                Wait for it to finish, or stop it with: 'kill {exec_pid}'",
            };
            assert_eq!(err.to_string(), expected_msg);

            stop_process(exec_proc);
        }
    }
}
