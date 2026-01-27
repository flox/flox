//! Event-driven PID monitoring using waitpid_any crate.
//!
//! This module provides event-driven process monitoring to replace the 100ms polling loop.
//! Each monitored PID gets a dedicated thread that:
//! - With expiration: Sleeps until expiration, then waits for process exit
//! - Without expiration: Immediately waits for process exit

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{Context, Result};
use flox_core::activations::read_activations_json;
use nix::libc::{SIGCHLD, SIGINT, SIGQUIT, SIGTERM, SIGUSR1};
use notify::{RecommendedWatcher, RecursiveMode, Watcher, recommended_watcher};
use signal_hook::iterator::Signals;
use time::OffsetDateTime;
use tracing::{debug, trace, warn};
use waitpid_any::WaitHandle;

/// Events that can occur during PID monitoring.
#[derive(Debug)]
pub enum PidEvent {
    /// A monitored process has exited
    ProcessExited { pid: i32 },
    /// A termination signal was received (SIGINT/SIGTERM/SIGQUIT)
    TerminationSignal,
    /// SIGCHLD received - need to reap orphaned children
    SigChld,
    /// SIGUSR1 received - start process-compose
    StartServices,
}

/// Coordinates PID monitoring across multiple threads.
pub struct PidMonitorCoordinator {
    sender: Sender<PidEvent>,
    pub receiver: Receiver<PidEvent>,
    known_pids: Arc<Mutex<HashSet<i32>>>,
}

impl PidMonitorCoordinator {
    /// Create a new coordinator with a channel for receiving events.
    pub fn new() -> Self {
        let (sender, receiver) = channel();
        Self {
            sender,
            receiver,
            known_pids: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// Start monitoring a PID. Spawns a dedicated thread for this PID.
    pub fn start_monitoring(&self, pid: i32, expiration: Option<OffsetDateTime>) {
        let mut known = self.known_pids.lock().unwrap();
        if known.contains(&pid) {
            trace!(pid, "PID already being monitored, skipping");
            return;
        }
        known.insert(pid);
        drop(known);

        spawn_pid_watcher(pid, expiration, self.sender.clone());
    }

    /// Start watching the state.json file for changes.
    /// When new PIDs are added, spawns watcher threads for them.
    pub fn start_state_watcher(&self, state_json_path: PathBuf) -> Result<RecommendedWatcher> {
        let sender = self.sender.clone();
        let known_pids = Arc::clone(&self.known_pids);
        let path_for_callback = state_json_path.clone();

        let mut watcher = recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
            match res {
                Ok(event) => {
                    if event.kind.is_modify() || event.kind.is_create() {
                        trace!(?event, "state.json changed, checking for new PIDs");
                        if let Ok((Some(state), _lock)) = read_activations_json(&path_for_callback)
                        {
                            let mut known = known_pids.lock().unwrap();
                            for (pid, expiration) in state.all_attached_pids_with_expiration() {
                                if !known.contains(&pid) {
                                    debug!(pid, "spawning watcher for new PID");
                                    known.insert(pid);
                                    spawn_pid_watcher(pid, expiration, sender.clone());
                                }
                            }
                        }
                    }
                },
                Err(e) => {
                    warn!(?e, "file watcher error");
                },
            }
        })
        .context("failed to create file watcher")?;

        watcher
            .watch(&state_json_path, RecursiveMode::NonRecursive)
            .context("failed to watch state.json")?;

        debug!(?state_json_path, "started watching state.json");
        Ok(watcher)
    }

    /// Start the signal handler thread.
    /// Returns a handle to the thread for cleanup.
    pub fn start_signal_handler(&self) -> Result<JoinHandle<()>> {
        let sender = self.sender.clone();

        let mut signals =
            Signals::new([SIGINT, SIGTERM, SIGQUIT, SIGCHLD, SIGUSR1]).context("failed to register signals")?;

        let handle = thread::spawn(move || {
            for signal in signals.forever() {
                let event = match signal {
                    SIGINT | SIGTERM | SIGQUIT => PidEvent::TerminationSignal,
                    SIGCHLD => PidEvent::SigChld,
                    SIGUSR1 => PidEvent::StartServices,
                    _ => continue,
                };
                debug!(signal, "received signal, sending event");
                if sender.send(event).is_err() {
                    // Channel closed, exit the thread
                    break;
                }
            }
        });

        debug!("started signal handler thread");
        Ok(handle)
    }

    /// Get a clone of the sender for testing or external use.
    #[cfg(test)]
    pub fn sender(&self) -> Sender<PidEvent> {
        self.sender.clone()
    }
}

/// Spawn a thread to watch a specific PID.
///
/// The thread will:
/// 1. If expiration exists and not yet passed: sleep until expiration
/// 2. Open a WaitHandle for the PID
/// 3. Block waiting for the process to exit
/// 4. Send ProcessExited event when done
fn spawn_pid_watcher(pid: i32, expiration: Option<OffsetDateTime>, sender: Sender<PidEvent>) {
    thread::spawn(move || {
        // Sleep until expiration if present and not yet passed
        if let Some(exp) = expiration {
            let now = OffsetDateTime::now_utc();
            if now < exp {
                let duration = (exp - now).unsigned_abs();
                let sleep_duration = duration.try_into().unwrap_or(Duration::MAX);
                debug!(pid, ?sleep_duration, "sleeping until expiration");
                thread::sleep(sleep_duration);
            }
        }

        // Now wait for process to exit
        let mut handle = match WaitHandle::open(pid) {
            Ok(h) => h,
            Err(e) => {
                // Process already dead or doesn't exist
                debug!(pid, ?e, "failed to open WaitHandle, process likely already dead");
                let _ = sender.send(PidEvent::ProcessExited { pid });
                return;
            },
        };

        debug!(pid, "waiting for process to exit");
        loop {
            match handle.wait() {
                Ok(()) => {
                    debug!(pid, "process exited");
                    let _ = sender.send(PidEvent::ProcessExited { pid });
                    return;
                },
                Err(e) if e.raw_os_error() == Some(nix::libc::EINTR) => {
                    // Interrupted by signal, retry
                    trace!(pid, "wait interrupted by signal, retrying");
                    continue;
                },
                Err(e) => {
                    // Some other error, treat as process exited
                    debug!(pid, ?e, "wait failed, treating as exited");
                    let _ = sender.send(PidEvent::ProcessExited { pid });
                    return;
                },
            }
        }
    });

    debug!(pid, ?expiration, "spawned PID watcher thread");
}

#[cfg(test)]
mod tests {
    use std::process::Command;
    use std::time::Duration;

    use super::*;

    #[test]
    fn test_pid_watcher_detects_exit() {
        let coordinator = PidMonitorCoordinator::new();

        // Start a short-lived process
        let child = Command::new("sleep")
            .arg("0.1")
            .spawn()
            .expect("failed to spawn");
        let pid = child.id() as i32;

        coordinator.start_monitoring(pid, None);

        // Wait for the event
        let event = coordinator
            .receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("should receive event");

        match event {
            PidEvent::ProcessExited { pid: exited_pid } => {
                assert_eq!(exited_pid, pid);
            },
            _ => panic!("expected ProcessExited event"),
        }
    }

    #[test]
    fn test_pid_watcher_already_dead_process() {
        let coordinator = PidMonitorCoordinator::new();

        // Use a PID that doesn't exist
        let fake_pid = 999999;

        coordinator.start_monitoring(fake_pid, None);

        // Should get ProcessExited immediately
        let event = coordinator
            .receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("should receive event");

        match event {
            PidEvent::ProcessExited { pid } => {
                assert_eq!(pid, fake_pid);
            },
            _ => panic!("expected ProcessExited event"),
        }
    }

    #[test]
    fn test_duplicate_monitoring_ignored() {
        let coordinator = PidMonitorCoordinator::new();

        // Start a long-running process
        let child = Command::new("sleep")
            .arg("10")
            .spawn()
            .expect("failed to spawn");
        let pid = child.id() as i32;

        coordinator.start_monitoring(pid, None);
        coordinator.start_monitoring(pid, None); // Should be ignored

        // Check that PID is only tracked once
        let known = coordinator.known_pids.lock().unwrap();
        assert!(known.contains(&pid));
        assert_eq!(known.len(), 1);
        drop(known);

        // Kill the process to clean up
        let _ = nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), nix::sys::signal::SIGKILL);
    }
}
