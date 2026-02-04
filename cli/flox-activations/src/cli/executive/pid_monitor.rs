//! This module replaces the polling-based monitoring loop with an event-driven
//! architecture.

use std::collections::HashMap;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use anyhow::{Context, Result, bail};
use flox_core::activations::{PidWithExpiration, read_activations_json};
use nix::libc::{SIGCHLD, SIGINT, SIGQUIT, SIGTERM};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use signal_hook::iterator::Signals;
use time::OffsetDateTime;
use tracing::{debug, error, trace, warn};
use waitpid_any::WaitHandle;

/// Events that can occur during PID monitoring.
#[derive(Debug, Clone)]
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
        self.ensure_monitoring_pids(activations.all_attached_pids_with_expiration())
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
        let mut known = self.known_pids.lock().unwrap();
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
        let state_json_path = state_json_path.as_ref();
        let parent_dir = state_json_path
            .parent()
            .context("state.json path has no parent directory")?
            .to_path_buf();
        let filename = state_json_path
            .file_name()
            .context("state.json path has no filename")?
            .to_owned();

        let mut watcher =
            notify::recommended_watcher(move |res: notify::Result<notify::Event>| match res {
                Ok(event) => {
                    // Filter for events affecting state.json
                    if !event
                        .paths
                        .iter()
                        .any(|p| p.file_name() == Some(filename.as_os_str()))
                    {
                        return;
                    }

                    debug!(?event, "state.json changed, sending event to main loop");

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

        debug!(state_json_path = %state_json_path.display(), "started watching state.json");
        Ok(watcher)
    }

    /// Stop monitoring a PID.
    ///
    /// Removes the PID from the known map and joins the watcher thread.
    /// This allows the PID to be re-monitored if needed (e.g., if it re-attached
    /// to the activation).
    pub fn stop_monitoring(&self, pid: i32) {
        let handle = {
            let mut known = self.known_pids.lock().unwrap();
            known.remove(&pid)
        };

        if let Some(handle) = handle {
            debug!(pid, "stopped monitoring PID, joining watcher thread");
            if handle.is_finished() {
                if let Err(err) = handle.join() {
                    error!(pid, ?err, "couldn't join watcher thread");
                }
            } else {
                error!(pid, "expected watcher thread for PID to be finished");
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

/// Spawn a thread that waits for a specific PID to exit.
///
/// If expiration is set, the thread sleeps until the expiration time before
/// starting to wait for the process to exit.
fn spawn_pid_watcher(
    pid: i32,
    expiration: Option<OffsetDateTime>,
    sender: Sender<ExecutiveEvent>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        // Try to open a wait handle for the process
        // Do this before sleeping to decrease the odds of a PID reuse race
        let mut handle = match WaitHandle::open(pid) {
            Ok(h) => h,
            Err(err) => {
                // Process likely already dead (ESRCH or similar)
                debug!(pid, %err, "failed to open wait handle, process likely already exited");
                let _ = sender.send(ExecutiveEvent::ProcessExited { pid });
                return;
            },
        };

        // Sleep until expiration if present and not yet passed
        if let Some(expiration) = expiration {
            let now = OffsetDateTime::now_utc();
            if now < expiration {
                let duration = (expiration - now).unsigned_abs();
                debug!(pid, ?duration, "sleeping until expiration");
                thread::sleep(duration);
            }
        }

        // Wait for the process to exit, retrying on EINTR
        loop {
            match handle.wait() {
                Ok(()) => {
                    debug!(pid, "process exited");
                    let _ = sender.send(ExecutiveEvent::ProcessExited { pid });
                    return;
                },
                Err(err) if err.raw_os_error() == Some(nix::libc::EINTR) => {
                    trace!(pid, "wait interrupted by signal, retrying");
                    continue;
                },
                Err(err) => {
                    // Unexpected error, treat as process exited
                    warn!(pid, %err, "unexpected error waiting for process");
                    let _ = sender.send(ExecutiveEvent::ProcessExited { pid });
                    return;
                },
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use std::process::Command;
    use std::time::Duration;

    use super::*;
}
