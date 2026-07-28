//! This module replaces the polling-based monitoring loop with an event-driven
//! architecture.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use flox_core::activations::{PidWithExpiration, read_activations_json};
use flox_core::proc_status::pid_is_running;

/// How long to wait between polling iterations when pidfd is unavailable.
const POLLING_INTERVAL: Duration = Duration::from_millis(100);
use nix::libc::{SIGCHLD, SIGINT, SIGQUIT, SIGTERM};
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use signal_hook::iterator::Signals;
use time::OffsetDateTime;
use tracing::{debug, error, trace, warn};
use waitpid_any::WaitHandle;

/// Events that can occur during PID monitoring.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExecutiveEvent {
    /// A monitored process has exited
    ProcessExited { pid: i32 },
    /// A termination signal was received (SIGINT/SIGTERM/SIGQUIT)
    TerminationSignal,
    /// SIGCHLD was received - reap orphaned children
    SigChld,
    /// SIGUSR1 was received - start process-compose
    StartServices,
    /// state.json was modified - check for new PIDs to monitor
    StateFileChanged,
}

/// Coordinates PID monitoring across multiple threads.
///
/// The coordinator maintains a channel for receiving events from:
/// - PID watcher threads (one per monitored PID)
/// - State file watcher (detects new PIDs added to state.json)
/// - Signal handler thread (SIGINT/SIGTERM/SIGQUIT/SIGCHLD/SIGUSR1)
#[derive(Debug)]
pub struct EventCoordinator {
    sender: Sender<ExecutiveEvent>,
    pub receiver: Receiver<ExecutiveEvent>,
    /// known_pids could have more PIDs than state.json (if e.g. multiple PIDs
    /// exit simultaneously)
    /// Or it could have fewer (if state.json has been updated but our notify
    /// event hasn't been handled yet)
    ///
    /// I don't think this currently needs to be protected with the Mutex,
    /// but I'll leave the Mutex for now in case threads other than main do
    /// start mutating it.
    known_pids: Arc<Mutex<HashMap<i32, JoinHandle<()>>>>,
    /// Handle to the signal handler thread (kept alive for the coordinator's lifetime)
    /// None for tests
    _signal_handler: Option<JoinHandle<()>>,
    /// Handle to the file watcher (kept alive for the coordinator's lifetime)
    /// None for tests
    _file_watcher: Option<RecommendedWatcher>,
}

impl EventCoordinator {
    /// Create a new coordinator and start monitoring.
    ///
    /// Reads initial state from state.json, starts monitoring existing PIDs,
    /// starts the file watcher for state changes, and starts the signal handler.
    pub fn new() -> Result<Self> {
        let (sender, receiver) = mpsc::channel();
        let known_pids = Arc::new(Mutex::new(HashMap::new()));

        Ok(Self {
            sender,
            receiver,
            known_pids,
            _signal_handler: None,
            _file_watcher: None,
        })
    }

    /// Spawns watchers all attached PIDs, state.json, and the signal
    /// handler.
    pub fn spawn_all_watchers(&mut self, state_json_path: impl AsRef<Path>) -> Result<()> {
        let (activations_json, _lock) = read_activations_json(&state_json_path)?;
        let Some(activations) = activations_json else {
            bail!("executive shouldn't be running when state.json doesn't exist");
        };

        // Watch attached PIDs
        self.ensure_monitoring_pids(activations.all_attached_pids_and_expiration())
            .context("failed to ensure monitoring PIDs")?;

        // Watch state.json
        let file_watcher = Self::start_state_watcher(state_json_path, self.sender.clone())
            .context("failed to start state file watcher")?;
        self._file_watcher = Some(file_watcher);

        // Start signal handler
        let signal_handler = Self::spawn_signal_handler(self.sender.clone())?;
        self._signal_handler = Some(signal_handler);

        Ok(())
    }

    /// Monitor PIDs not already monitored.
    /// This is idempotent.
    pub fn ensure_monitoring_pids(
        &self,
        pids_with_expiration: Vec<PidWithExpiration>,
    ) -> Result<()> {
        for (pid, expiration) in pids_with_expiration {
            self.start_monitoring(pid, expiration)?;
        }
        Ok(())
    }

    /// Start monitoring a PID.
    ///
    /// Spawns a thread that waits for the process to exit. If expiration is set,
    /// the thread will sleep until the expiration time before starting to wait.
    ///
    /// This is idempotent.
    pub fn start_monitoring(&self, pid: i32, expiration: Option<OffsetDateTime>) -> Result<()> {
        let mut known = self
            .known_pids
            .lock()
            .expect("another thread panicked while holding the mutex");
        if known.contains_key(&pid) {
            trace!(pid, "PID already being monitored, skipping");
            return Ok(());
        }

        let sender = self.sender.clone();
        let handle = spawn_pid_watcher(pid, expiration, sender);
        known.insert(pid, handle);
        debug!(pid, ?expiration, "started monitoring PID");
        Ok(())
    }

    /// Start watching state.json for changes.
    ///
    /// Returns a watcher that must be kept alive for the duration of monitoring.
    /// The watcher sends `StateFileChanged` events to the main loop when modifications
    /// are detected. The main loop is responsible for reading the state and spawning
    /// watchers for new PIDs.
    ///
    /// We watch the parent directory rather than the file directly because state.json
    /// is written atomically via rename, which doesn't produce modify events on the
    /// target file.
    ///
    /// The callback function is called without us having to manage a separate thread.
    fn start_state_watcher(
        state_json_path: impl AsRef<Path>,
        sender: Sender<ExecutiveEvent>,
    ) -> Result<RecommendedWatcher> {
        let owned_state_json_path = state_json_path.as_ref().to_path_buf();
        let parent_dir = owned_state_json_path
            .parent()
            .context("state.json path has no parent directory")?
            .to_path_buf();
        let filename = owned_state_json_path
            .file_name()
            .context("state.json path has no filename")?
            .to_owned();

        let mut watcher =
            notify::recommended_watcher(move |res: notify::Result<notify::Event>| match res {
                Ok(event) => {
                    if !should_emit_state_changed(&event, &owned_state_json_path, &filename) {
                        return;
                    }

                    // Warn if the event is reporting events are being dropped
                    if event.need_rescan() {
                        warn!(?event, "file watcher dropped events, sending event to main loop to double check for state.json changes");
                    } else {
                        debug!(?event, "state.json changed, sending event to main loop");
                    }

                    if sender.send(ExecutiveEvent::StateFileChanged).is_err() {
                        // Channel closed, nothing to do
                        error!("failed to send StateFileChanged event, channel closed");
                    }
                },
                Err(err) => {
                    error!(%err, "file watcher error");
                },
            })
            .context("failed to create file watcher")?;

        watcher
            .watch(&parent_dir, RecursiveMode::NonRecursive)
            .context("failed to watch state.json parent directory")?;

        debug!(state_json_path = %state_json_path.as_ref().display(), "started watching state.json");
        Ok(watcher)
    }

    /// Stop monitoring a PID.
    ///
    /// Removes the PID from the known map and joins the watcher thread.
    /// This allows the PID to be re-monitored if needed (e.g., if it re-attached
    /// to the activation).
    pub fn stop_monitoring(&self, pid: i32) {
        let handle = {
            let mut known = self
                .known_pids
                .lock()
                .expect("another thread panicked while holding the mutex");
            known.remove(&pid)
        };

        if let Some(handle) = handle {
            debug!(pid, "stopped monitoring PID, joining watcher thread");
            if handle.is_finished() {
                if let Err(err) = handle.join() {
                    warn!(pid, ?err, "couldn't join watcher thread");
                }
            } else {
                // Common case: thread sent ProcessExited as its last action and
                // is finishing its stack unwind. Release the handle; the thread
                // completes independently in microseconds.
                debug!(pid, "watcher thread not yet finished, releasing handle");
            }
        } else {
            error!(pid, "stop_monitoring called for PID not in known set");
        }
    }

    /// Inject an event into the coordinator for testing.
    ///
    /// This allows tests to simulate events without relying on real signals
    /// or process exits.
    #[cfg(test)]
    pub fn inject_event(&self, event: ExecutiveEvent) {
        let _ = self.sender.send(event);
    }

    /// Spawn signal handler thread.
    ///
    /// Returns the thread handle.
    fn spawn_signal_handler(sender: Sender<ExecutiveEvent>) -> Result<JoinHandle<()>> {
        let handle = thread::spawn(move || {
            // WARNING: You cannot reliably use Command::wait after SignalHandlers is
            // created, including concurrent threads like GCing logs, because children
            // will be reaped automatically.
            let mut signals =
                match Signals::new([SIGINT, SIGTERM, SIGQUIT, SIGCHLD, nix::libc::SIGUSR1]) {
                    Ok(s) => s,
                    Err(err) => {
                        error!(%err, "failed to register signals");
                        return;
                    },
                };

            for signal in signals.forever() {
                let event = match signal {
                    SIGINT | SIGTERM | SIGQUIT => {
                        debug!(signal, "received termination signal");
                        ExecutiveEvent::TerminationSignal
                    },
                    SIGCHLD => {
                        debug!("received SIGCHLD");
                        ExecutiveEvent::SigChld
                    },
                    nix::libc::SIGUSR1 => {
                        debug!("received SIGUSR1 (start services)");
                        ExecutiveEvent::StartServices
                    },
                    _ => continue,
                };

                if sender.send(event).is_err() {
                    // Channel closed, exit the thread
                    break;
                }
            }
        });

        debug!("started signal handler thread");
        Ok(handle)
    }
}

/// Whether a watcher event means StateFileChanged should be sent to the main loop.
///
/// Emit StateFileChanged for:
/// - File creation (atomic rename) or data modification.
///   This filters out Close, Access, Remove, and other irrelevant events
///   that would cause redundant state.json reads.
/// - A rescan event, in which case the watcher is dropping events. In that case
///   we have to double check state.json exists because the main loop assumes it
///   does
fn should_emit_state_changed(event: &Event, state_json_path: &Path, filename: &OsStr) -> bool {
    let is_write_event = event.kind.is_modify() || event.kind.is_create();
    let is_state_json_event = event.paths.iter().any(|p| p.file_name() == Some(filename));
    if is_write_event && is_state_json_event {
        return true;
    }

    if event.need_rescan() && state_json_path.exists() {
        return true;
    }

    false
}

/// Spawn a thread that waits for a specific PID to exit.
///
/// If expiration is set, the thread sleeps until the expiration time before
/// starting to wait for the process to exit.
///
/// On Linux < 5.3 where pidfd_open is unavailable (returns ENOSYS), falls back
/// to polling the process status every 100ms.
fn spawn_pid_watcher(
    pid: i32,
    expiration: Option<OffsetDateTime>,
    sender: Sender<ExecutiveEvent>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        // Try to open a wait handle for the process
        // Do this before sleeping to decrease the odds of a PID reuse race
        let handle_result = WaitHandle::open(pid);

        // Sleep until expiration if present and not yet passed
        // Do this even if the PID has already exited so we don't immediately
        // start re-monitoring the PID
        if let Some(expiration) = expiration {
            let now = OffsetDateTime::now_utc();
            if now < expiration {
                let duration = (expiration - now).unsigned_abs();
                debug!(pid, ?duration, "sleeping until expiration");
                thread::sleep(duration);
            }
        }

        let handle = match handle_result {
            Ok(h) => Some(h),
            Err(err) if err.raw_os_error() == Some(nix::libc::ENOSYS) => {
                // pidfd_open not available (Linux < 5.3), will use polling fallback
                warn!(pid, "pidfd unavailable (ENOSYS), using polling fallback");
                None
            },
            Err(err) => {
                // Process likely already dead (ESRCH or similar)
                debug!(pid, %err, "failed to open wait handle, process likely already exited");
                // Ignore error since we're just going to return
                let _ = sender.send(ExecutiveEvent::ProcessExited { pid });
                return;
            },
        };

        // Wait for the process to exit
        if let Some(mut handle) = handle {
            debug!("waiting for PID {} blocking", pid);
            // Use pidfd-based wait
            loop {
                match handle.wait() {
                    Ok(()) => {
                        debug!(pid, "process exited");
                        // Ignore error since we're just going to return
                        let _ = sender.send(ExecutiveEvent::ProcessExited { pid });
                        return;
                    },
                    Err(err) if err.raw_os_error() == Some(nix::libc::EINTR) => {
                        trace!(pid, "wait interrupted by signal, retrying");
                        continue;
                    },
                    Err(err) => {
                        warn!(pid, %err, "unexpected error waiting for process");
                        // Ignore error since we're just going to return
                        let _ = sender.send(ExecutiveEvent::ProcessExited { pid });
                        return;
                    },
                }
            }
        } else {
            debug!(
                "polling for PID {} to exit on system without blocking support",
                pid
            );
            loop {
                if !pid_is_running(pid) {
                    debug!(pid, "process exited (polling)");
                    // Ignore error since we're just going to return
                    let _ = sender.send(ExecutiveEvent::ProcessExited { pid });
                    return;
                }
                thread::sleep(POLLING_INTERVAL);
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::Duration;

    use flox_core::write_atomically;

    use super::*;
    use crate::cli::executive::watcher::test::{start_process, stop_process};

    /// After setting up a coordinator and calling ensure_monitoring_pids,
    /// a ProcessExited event is sent when a PID exits.
    #[test]
    fn pid_exit_sends_process_exited_event() {
        let coordinator = EventCoordinator::new().unwrap();

        let proc = start_process();
        let pid = proc.id() as i32;

        // Monitor the PID
        coordinator
            .ensure_monitoring_pids(vec![(pid, None)])
            .unwrap();

        stop_process(proc);

        // Receive the event - should arrive within a reasonable timeout
        let event = coordinator
            .receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("should receive event within timeout");

        assert_eq!(event, ExecutiveEvent::ProcessExited { pid });
    }

    /// State watcher sends a StateFileChanged event when state.json is modified.
    #[test]
    fn state_watcher_sends_event_on_state_file_change() {
        let temp_dir = tempfile::tempdir().unwrap();
        let state_json_path = temp_dir.path().join("state.json");

        // Create initial state.json file
        fs::write(&state_json_path, "{}").unwrap();

        // Set up channel and start the state watcher
        let (sender, receiver) = mpsc::channel();
        let _watcher = EventCoordinator::start_state_watcher(&state_json_path, sender)
            .expect("failed to start state watcher");

        write_atomically(&state_json_path, "{\"modified\": true}").unwrap();

        // Receive the event - should arrive within a reasonable timeout
        let event = receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("should receive event within timeout");

        assert_eq!(event, ExecutiveEvent::StateFileChanged);
    }

    mod should_emit_state_changed {
        use std::path::PathBuf;

        use notify::EventKind;
        use notify::event::{CreateKind, Flag, ModifyKind, RenameMode};

        use super::*;

        /// A directory holding a state.json, plus the path to it.
        fn state_json_in_temp_dir() -> (tempfile::TempDir, PathBuf) {
            let temp_dir = tempfile::tempdir().unwrap();
            let state_json_path = temp_dir.path().join("state.json");
            fs::write(&state_json_path, "{}").unwrap();
            (temp_dir, state_json_path)
        }

        fn rescan_event() -> Event {
            Event::new(EventKind::Other).set_flag(Flag::Rescan)
        }

        /// state.json is written by atomic rename, so the rename onto the
        /// target name is the event that means the state changed.
        #[test]
        fn rename_onto_state_json_is_a_change() {
            let (_temp_dir, state_json_path) = state_json_in_temp_dir();
            let event = Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::Any)))
                .add_path(state_json_path.clone());

            assert!(should_emit_state_changed(
                &event,
                &state_json_path,
                state_json_path.file_name().unwrap()
            ));
        }

        /// The temp file the atomic write lands in first is not state.json;
        /// re-reading on it would just read the previous contents.
        #[test]
        fn temp_file_create_is_not_a_change() {
            let (temp_dir, state_json_path) = state_json_in_temp_dir();
            let event = Event::new(EventKind::Create(CreateKind::File))
                .add_path(temp_dir.path().join(".tmpAbC123"));

            assert!(!should_emit_state_changed(
                &event,
                &state_json_path,
                state_json_path.file_name().unwrap()
            ));
        }

        /// A rescan is the backend saying it dropped events. It names no file
        /// and its kind is neither create nor modify, so filtering on kind and
        /// name alone would discard it — and with it any attach that happened
        /// during the overflow, leaving that PID unmonitored forever.
        #[test]
        fn rescan_is_a_change_despite_naming_no_file() {
            let (_temp_dir, state_json_path) = state_json_in_temp_dir();
            let event = rescan_event();

            assert!(
                event.paths.is_empty(),
                "a rescan carries no path to match on"
            );
            assert!(
                !event.kind.is_modify() && !event.kind.is_create(),
                "a rescan is neither a modify nor a create"
            );
            assert!(should_emit_state_changed(
                &event,
                &state_json_path,
                state_json_path.file_name().unwrap()
            ));
        }

        /// A rescan carries no timing guarantee, so it can arrive after the
        /// activation was torn down. Signalling a re-read then would have the
        /// loop treat the missing file as fatal, and reading it would recreate
        /// the directory cleanup had just removed.
        #[test]
        fn rescan_after_state_json_is_gone_is_not_a_change() {
            let (_temp_dir, state_json_path) = state_json_in_temp_dir();
            fs::remove_file(&state_json_path).unwrap();

            assert!(!should_emit_state_changed(
                &rescan_event(),
                &state_json_path,
                state_json_path.file_name().unwrap()
            ));
        }
    }
}
