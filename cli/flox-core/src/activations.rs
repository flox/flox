use std::path::{Path, PathBuf};

use anyhow::Context;
use fslock::LockFile;
use serde::{Deserialize, Serialize};
use serde_json::json;
use time::OffsetDateTime;
use tracing::debug;

use crate::path_hash;
use crate::proc_status::pid_is_running;

type Error = anyhow::Error;

/// Latest supported version for compatibility between:
/// - `flox` and `flox-interpreter`
/// - `flox-activations` and `flox-watchdog`
///
/// Incrementing this will require existing activations to exit.
const LATEST_VERSION: u8 = 2;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct UncheckedVersion(u8);
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CheckedVersion(u8);
impl Default for CheckedVersion {
    fn default() -> Self {
        Self(LATEST_VERSION)
    }
}

#[derive(Debug, Eq, PartialEq, thiserror::Error)]
#[error(
    "This environment has already been activated with an incompatible version of 'flox'.\n\
     \n\
     Exit all activations of the environment and try again.\n\
     PIDs of the running activations: {pid_list}",
    pid_list = .pids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(", "))]
pub struct Unsupported {
    pub version: UncheckedVersion,
    pub pids: Vec<i32>,
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

/// Acquires the filesystem-based lock on activations.json
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

/// Returns the path to the lock file for activations.json.
/// The presence of the lock file does not indicate an active lock because the
/// file isn't removed after use.
/// This is a separate file because we replace activations.json on write.
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

#[allow(dead_code)] // TODO
pub mod rewrite {
    use std::collections::BTreeMap;
    use std::ops::Deref;

    use super::*;
    use crate::Version;
    use crate::activate::mode::ActivateMode;

    type Pid = i32;

    const EXECUTIVE_NOT_STARTED: Pid = 0;

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub enum StartOrAttachResult {
        /// A new activation was started for the given StartIdentifier
        Start {
            start_id: StartIdentifier,
            needs_new_executive: bool,
        },
        /// Attached to an existing ready activation with the given StartIdentifier
        Attach {
            start_id: StartIdentifier,
            needs_new_executive: bool,
        },
        /// Another process is currently starting an activation.
        /// The caller should wait and retry.
        AlreadyStarting { pid: Pid, start_id: StartIdentifier },
    }

    #[derive(
        Clone, Debug, Deserialize, derive_more::Display, Eq, PartialEq, Serialize, Ord, PartialOrd,
    )]
    pub struct UnixTimestamp(i64);

    impl UnixTimestamp {
        pub fn now() -> Self {
            Self(OffsetDateTime::now_utc().unix_timestamp())
        }
    }

    impl Deref for UnixTimestamp {
        type Target = i64;

        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }

    impl std::str::FromStr for UnixTimestamp {
        type Err = std::num::ParseIntError;

        fn from_str(s: &str) -> Result<Self, Self::Err> {
            s.parse::<i64>().map(UnixTimestamp)
        }
    }

    #[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, Ord, PartialOrd)]
    pub struct StartIdentifier {
        pub store_path: PathBuf,
        pub timestamp: UnixTimestamp,
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
            let base_dir = super::activation_state_dir_path(runtime_dir, dot_flox_path);
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

    #[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
    pub struct ActivationState {
        // TODO: How to handle upgrades
        version: Version<3>,

        // TODO: Group in "info", but restricts how we might use them in future?
        // dot_flox_path: PathBuf,
        // flox_env: PathBuf,
        mode: ActivateMode,
        ready: Ready,
        executive_pid: Pid,
        current_process_compose_store_path: Option<StartIdentifier>,
        attached_pids: BTreeMap<Pid, Attachment>,
    }

    impl ActivationState {
        pub fn new(mode: &ActivateMode) -> Self {
            Self {
                version: Version,
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

        /// Returns a mapping of StartIdentifier to the list of PIDs attached to it.
        pub fn attached_pids_by_start_id(&self) -> BTreeMap<StartIdentifier, Vec<Pid>> {
            self.attached_pids
                .iter()
                .fold(BTreeMap::new(), |mut acc, (pid, attachment)| {
                    acc.entry(attachment.start_id.clone())
                        .or_default()
                        .push(*pid);
                    acc
                })
        }

        pub fn attached_pids_is_empty(&self) -> bool {
            self.attached_pids.is_empty()
        }

        // TODO: used in tests only
        pub fn attached_pids(&self) -> Vec<Pid> {
            self.attached_pids.keys().copied().collect()
        }

        /// Returns the current activation mode
        pub fn mode(&self) -> &ActivateMode {
            &self.mode
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

            let needs_new_executive = self.needs_new_executive();
            let ready = self.ready.clone();
            match ready {
                Ready::True(start_id) if start_id.store_path == store_path.as_ref() => {
                    self.attach(pid, Attachment {
                        start_id: start_id.clone(),
                        expiration: None,
                    });
                    StartOrAttachResult::Attach {
                        start_id,
                        needs_new_executive,
                    }
                },
                Ready::False | Ready::True(_) | Ready::Starting(_, _) => {
                    let start_id = self.start(pid, &store_path);
                    StartOrAttachResult::Start {
                        start_id,
                        needs_new_executive,
                    }
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

        /// set ready to False if there are no more PIDs attached to the current start
        /// should only be called when there are some attached PIDs
        pub fn update_ready_after_detach(&mut self) {
            if self.attached_pids.is_empty() {
                unreachable!("should remove all state when there are no more attached PIDs");
            }
            match self.ready {
                Ready::True(ref start_id) => {
                    if !self.attached_pids_by_start_id().contains_key(start_id) {
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
                timestamp: UnixTimestamp::now(),
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

        /// Check if the executive needs to be spawned.
        fn needs_new_executive(&self) -> bool {
            if self.executive_pid == EXECUTIVE_NOT_STARTED {
                debug!("executive has not been spawned yet");
                return true;
            }

            if !pid_is_running(self.executive_pid) {
                debug!(pid = self.executive_pid, "executive process is not running");
                return true;
            }

            debug!(pid = self.executive_pid, "executive process is running");
            false
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

    pub fn read_activations_json(
        path: impl AsRef<Path>,
    ) -> Result<(Option<ActivationState>, LockFile), Error> {
        let path = path.as_ref();
        let lock_file =
            acquire_activations_json_lock(path).context("failed to acquire lockfile")?;

        if !path.exists() {
            debug!("activations file not found at {}", path.to_string_lossy());
            return Ok((None, lock_file));
        }

        // TODO: version check

        debug!(?path, "reading activations.json");
        let contents = std::fs::read_to_string(path)
            .context(format!("failed to read file {}", path.display()))?;
        let parsed: ActivationState = serde_json::from_str(&contents)
            .context(format!("failed to parse JSON from {}", path.display()))?;
        Ok((Some(parsed), lock_file))
    }

    pub fn write_activations_json(
        activations: &ActivationState,
        path: impl AsRef<Path>,
        lock: LockFile,
    ) -> Result<(), Error> {
        crate::serialize_atomically(&json!(activations), &path, lock)?;
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use std::process::{Child, Command};

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

        fn make_activations(ready: Ready) -> rewrite::ActivationState {
            rewrite::ActivationState {
                version: Version,
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
                timestamp: UnixTimestamp::now(),
            }
        }

        fn make_attachment(start_id: StartIdentifier) -> Attachment {
            Attachment {
                start_id,
                expiration: None,
            }
        }

        mod attached_pids_getters {
            use super::*;

            #[test]
            fn test_attached_pids_running() {
                let proc_running = start_process();
                let proc_stopped = start_process();

                let mut activations = ActivationState::new(&ActivateMode::default());
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
                let mut activations = ActivationState::new(&ActivateMode::default());
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

                let expected =
                    BTreeMap::from([(start_id1, vec![100, 200]), (start_id2, vec![300])]);
                assert_eq!(
                    activations.attached_pids_by_start_id(),
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
                    StartOrAttachResult::Start {
                        start_id,
                        needs_new_executive: needs_executive_spawn,
                    } => {
                        assert!(
                            !needs_executive_spawn,
                            "Executive should not need spawning (PID 1 always runs)"
                        );
                        start_id
                    },
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
                    StartOrAttachResult::Attach {
                        start_id: id,
                        needs_new_executive: needs_executive_spawn,
                    } => {
                        assert_eq!(id, start_id);
                        assert!(
                            !needs_executive_spawn,
                            "Executive should already be running"
                        );
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
                    StartOrAttachResult::Start {
                        start_id,
                        needs_new_executive: needs_executive_spawn,
                    } => {
                        assert!(
                            !needs_executive_spawn,
                            "Executive should already be running"
                        );
                        start_id
                    },
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
                    StartOrAttachResult::Start {
                        start_id,
                        needs_new_executive: needs_executive_spawn,
                    } => {
                        assert!(
                            !needs_executive_spawn,
                            "Executive should not need spawning (PID 1 always runs)"
                        );
                        start_id
                    },
                    _ => panic!("Expected StartOrAttachResult::Start, got {:?}", result),
                };

                let (ready_pid, ready_start_id) = match &activations.ready {
                    Ready::Starting(p, s) => (*p, s.clone()),
                    _ => panic!("Expected Ready::Starting"),
                };
                assert_eq!(ready_pid, pid);
                assert_eq!(new_start_id, ready_start_id);
                // TODO: timestamps only have a resolution of 1 second so these are currently equal
                // assert!(new_start_id.timestamp > old_start_id.timestamp);
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
                        StartOrAttachResult::Attach {
                            start_id: id,
                            needs_new_executive: needs_executive_spawn,
                        } => {
                            assert_eq!(id, start_id);
                            assert!(
                                !needs_executive_spawn,
                                "Executive should already be running"
                            );
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
                let mut activations = ActivationState::new(&ActivateMode::default());
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
                    StartOrAttachResult::Attach {
                        start_id: id,
                        needs_new_executive: needs_executive_spawn,
                    } => {
                        assert_eq!(id, start_id);
                        assert!(
                            !needs_executive_spawn,
                            "Executive should already be running"
                        );
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
    }
}
