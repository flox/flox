//! This module contains the functionality that listens for signals and process termination.
//!
//! On macOS this can be configured to listen for the termination of any process, but on Linux
//! since we're using `prctl(2)` we are restricted to listening for the termination of the watchdog's
//! parent process.
//!
//! On Linux `prctl(2)` is configured to deliver a user-specified signal (we chose SIGUSR1) when
//! the target process terminates. This means that waiting for the notification is nonblocking on
//! Linux. On macOS *waiting* for termination is blocking, but since we need to periodically check
//! the shutdown flag, we use a poll-based method instead.
//!
//! Since we need to wait for a signal on Linux, we spawn a task that completes when a signal has
//! been delivered. We also install a signal handler on macOS for feature parity. Depending on which
//! signal is delivered we either trigger cleanup or simply shut down.
//!
//! On macOS since the termination notification is not delivered via signal, we need to spawn a task
//! that will only complete once the notification has arrived. For compatibility between macOS and
//! Linux implementations we also spawn a task on Linux, but this task is configured to never
//! complete (otherwise the watchdog would exit before the signal is delivered). The task on macOS
//! is created via `spawn_blocking` because there are no async calls in this task, but we still
//! want the task to run in concert with the other async tasks (this is why we don't use a thread).

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use futures::{select, FutureExt, StreamExt};
use nix::unistd::Pid;
use signal_hook_tokio::Signals;
use tokio::task::JoinHandle;
use tracing::{debug, debug_span, error, info_span, Instrument};

use crate::Error;

const FLAG_CHECK_INTERVAL: Duration = Duration::from_millis(10);

/// Produces a future that resolves when the shutdown flag is set.
pub async fn wait_for_shutdown(flag: Arc<AtomicBool>) {
    while !flag.load(std::sync::atomic::Ordering::SeqCst) {
        tokio::time::sleep(FLAG_CHECK_INTERVAL).await;
    }
}

/// Returns the PID that will be waited on
pub fn target_pid(args: &crate::Cli) -> Pid {
    #[cfg(target_os = "linux")]
    let pid = if let Some(_pid) = args.pid {
        debug!("ignoring user-provided PID, not available on Linux, using parent PID instead");
        nix::unistd::getppid()
    } else {
        debug!("using parent PID for target PID");
        nix::unistd::getppid()
    };

    #[cfg(target_os = "macos")]
    let pid = if let Some(pid) = args.pid {
        debug!("using user-provided target PID");
        Pid::from_raw(pid)
    } else {
        debug!("using parent PID for target PID");
        nix::unistd::getppid()
    };

    pid
}

/// What should be done in response to receiving a signal
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Action {
    /// Shut down the watchdog
    Terminate,
    /// Trigger service cleanup
    Cleanup,
}

impl std::fmt::Display for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Action::Terminate => write!(f, "terminate"),
            Action::Cleanup => write!(f, "cleanup"),
        }
    }
}

/// Spawns a task that resolves on the delivery of a signal of interest.
pub(crate) fn spawn_signal_listener(
    shutdown_flag: Arc<AtomicBool>,
) -> Result<JoinHandle<Result<Action, Error>>, Error> {
    use signal_hook::consts::signal::*;
    let mut signals = Signals::new([SIGTERM, SIGINT, SIGQUIT, SIGUSR1])
        .context("couldn't install signal handler")?;
    // Creates a stream of signals that were deliverd to the process
    let signals_stream_handle = signals.handle();
    Ok(tokio::spawn(async move {
        let span = debug_span!("signal_listener");
        let _ = span.enter();
        debug!(task = "signal_listener", "task started");
        let action = select! {
            maybe_signal = signals
                .next()
                .instrument(info_span!("wait_for_signal")).fuse()
                => {
                if let Some(signal) = maybe_signal {
                    match signal {
                        // Signals that should trigger shutdown
                        SIGTERM | SIGINT | SIGQUIT => {
                            debug!("received shutdown signal");
                            Action::Terminate
                        },
                        // Signal that should trigger cleanup
                        SIGUSR1 => {
                            debug!("received cleanup signal");
                            Action::Cleanup
                        },
                        // We didn't install a signal handler for any other signals
                        _ => unreachable!(),
                    }
                } else {
                    error!("signal stream was terminated");
                    Action::Terminate
                }
            },
            _ = wait_for_shutdown(shutdown_flag).fuse() => {
                debug!(task = "signal_listener", "observed shutdown flag");
                Action::Terminate
            }
        };
        signals_stream_handle.close();
        Ok(action)
    }))
}

/// Spawns a task that registers a listener via `kqueue` and waits for notification
/// that the target PID has terminated.
#[cfg(target_os = "macos")]
pub(crate) fn spawn_termination_listener(
    pid: Pid,
    shutdown_flag: Arc<AtomicBool>,
) -> JoinHandle<Result<Action, Error>> {
    use std::sync::atomic::Ordering;
    // NOTE: You cannot call `.abort` on this task because it is spawned with `spawn_blocking`,
    //       and attempting to do so will have no effect, that's why we need a shutdown flag.
    tokio::task::spawn_blocking(move || {
        let span = debug_span!("macos_termination_listener");
        let _ = span.enter();
        debug!(task = "macos_termination_listener", "task started");
        let mut watcher = kqueue::Watcher::new()?;
        watcher.add_pid(
            pid.into(),
            kqueue::EventFilter::EVFILT_PROC,
            kqueue::FilterFlag::NOTE_EXIT,
        )?;
        let action = loop {
            // The only event coming our way is the exit event for
            // the parent pid, so just grab it and continue.
            if let Some(_event) = watcher.poll(None) {
                debug!("received termination event");
                break Action::Cleanup;
            }
            if shutdown_flag.load(Ordering::SeqCst) {
                debug!(
                    task = "macos_termination_listener",
                    "observed shutdown flag"
                );
                break Action::Terminate;
            }
            std::thread::sleep(FLAG_CHECK_INTERVAL);
        };
        Ok(action)
    })
}

/// Spawns a task that registers this process via `prctl(2)` to be notified that its parent
/// has terminated.
#[cfg(target_os = "linux")]
pub(crate) fn spawn_termination_listener(
    _pid: Pid,
    shutdown_flag: Arc<AtomicBool>,
) -> JoinHandle<Result<Action, Error>> {
    tokio::spawn(async {
        let span = debug_span!("linux_termination_listener");
        let _ = span.enter();
        debug!(task = "linux_termination_listener", "task started");
        nix::sys::prctl::set_pdeathsig(Some(nix::sys::signal::Signal::SIGUSR1))
            .context("set_pdeathsig failed")?;
        wait_for_shutdown(shutdown_flag).await;
        debug!(
            task = "linux_termination_listener",
            "observed shutdown flag"
        );
        Ok(Action::Terminate)
    })
}
