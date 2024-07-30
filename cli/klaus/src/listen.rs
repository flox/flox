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

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use futures::future::Either;
use futures::{select, FutureExt, StreamExt};
use nix::unistd::Pid;
use signal_hook::consts::signal::*;
use signal_hook_tokio::{Handle, Signals};
use tokio::task::JoinHandle;
use tracing::{debug, debug_span, error, info, info_span, Instrument};

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

/// Behavior for something that can listen for signals
trait Listen {
    /// Listen for the first signal to be delivered
    async fn listen(&mut self) -> Option<i32>;
    /// Clean up any resources (only really needed for the real listener)
    fn close(&mut self);
}

/// A type that can listen for signals
pub enum SignalListener {
    Real(RealSignalListener),
    #[allow(dead_code)] // used in tests
    Mock(mock::MockListener),
}

impl Listen for SignalListener {
    async fn listen(&mut self) -> Option<i32> {
        match self {
            SignalListener::Real(listener) => listener.listen().await,
            SignalListener::Mock(listener) => listener.listen().await,
        }
    }

    fn close(&mut self) {
        match self {
            SignalListener::Real(listener) => listener.close(),
            SignalListener::Mock(listener) => listener.close(),
        }
    }
}

/// Listens for a signal delivered to the process
pub struct RealSignalListener {
    signals: Signals, // this doesn't impl Debug...somehow
    handle: Handle,
}

impl RealSignalListener {
    pub fn new(signals: Signals) -> Self {
        let handle = signals.handle();
        Self { signals, handle }
    }
}

impl Listen for RealSignalListener {
    async fn listen(&mut self) -> Option<i32> {
        self.signals
            .next()
            .instrument(info_span!("signal_listener"))
            .await
    }

    fn close(&mut self) {
        self.handle.close();
    }
}

// Doing this makes the `MockListener` fields private, meaning that you can't construct a `MockListener`
// except via the `MockListener::new` function, which is only available when `cfg(test)`
mod mock {
    use super::*;

    /// Delivers a configurable signal once a flag is set
    #[derive(Debug, Default)]
    pub struct MockListener {
        /// A flag used to indicate that it's time for the signal to be delivered
        flag: Arc<AtomicBool>,
        /// The signal that will be delivered
        sig: Option<i32>,
    }

    impl MockListener {
        #[cfg(test)]
        pub fn new(sig: Option<i32>, flag: Arc<AtomicBool>) -> Self {
            Self { sig, flag }
        }
    }

    impl Listen for MockListener {
        async fn listen(&mut self) -> Option<i32> {
            loop {
                if self.flag.load(Ordering::SeqCst) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            self.sig
        }

        // Nothing to close in this case
        fn close(&mut self) {}
    }
}

pub fn signal_listener() -> Result<SignalListener, Error> {
    let signals = Signals::new([SIGTERM, SIGINT, SIGQUIT, SIGUSR1])
        .context("couldn't install signal handler")?;
    Ok(SignalListener::Real(RealSignalListener::new(signals)))
}

/// Spawns a task that resolves on the delivery of a signal of interest.
pub(crate) fn spawn_signal_listener(
    mut signal_listener: SignalListener,
    shutdown_flag: Arc<AtomicBool>,
) -> Result<JoinHandle<Result<Action, Error>>, Error> {
    Ok(tokio::spawn(async move {
        let span = debug_span!("signal_listener");
        let _ = span.enter();
        debug!(task = "signal_listener", "task started");
        let action = select! {
            maybe_signal = signal_listener.listen().fuse()
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
        signal_listener.close();
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
        watcher.watch().context("failed to register watcher")?;
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

/// Waits for a notification (signal, termination, or error) and returns
/// which action should be taken in response to the notification.
pub async fn listen(
    sig_task: JoinHandle<Result<Action, Error>>,
    term_task: JoinHandle<Result<Action, Error>>,
    shutdown: Arc<AtomicBool>,
) -> Action {
    match futures::future::select(term_task, sig_task).await {
        Either::Left((maybe_term_action, unresolved_signal_task)) => {
            info!("received termination, setting shutdown flag");
            shutdown.store(true, Ordering::SeqCst);
            // Let the signal task shut down gracefully
            debug!("waiting for signal task to abort");
            let _ = unresolved_signal_task.await;
            match maybe_term_action {
                Ok(Ok(action)) => {
                    debug!(%action, "termination task completed successfully");
                    action
                },
                Ok(Err(err)) => {
                    error!(%err, "error encountered in termination task");
                    Action::Terminate
                },
                Err(err) => {
                    error!(%err, "termination task was cancelled");
                    Action::Terminate
                },
            }
        },
        Either::Right((maybe_signal_action, unresolved_termination_task)) => {
            info!("received signal, setting shutdown flag");
            shutdown.store(true, Ordering::SeqCst);
            // Let the signal task shut down gracefully
            debug!("waiting for termination task to shut down");
            let _ = unresolved_termination_task.await;
            match maybe_signal_action {
                Ok(Ok(action)) => {
                    debug!(%action, "signal task completed successfully");
                    action
                },
                Ok(Err(err)) => {
                    error!(%err, "error encountered in signal task");
                    Action::Terminate
                },
                Err(err) => {
                    error!(%err, "signal task was cancelled");
                    Action::Terminate
                },
            }
        },
    }
}

#[cfg(test)]
mod test {
    use std::task::Context;

    use futures::task::noop_waker_ref;

    use super::*;

    /// Returns a [std::task::Context] that does nothing,
    /// only useful for testing purposes when you need to poll a future
    /// without scheduling it to be woken up again.
    fn dummy_ctx() -> Context<'static> {
        Context::from_waker(noop_waker_ref())
    }

    #[tokio::test]
    async fn shutdown_flag_works() {
        let flag = Arc::new(AtomicBool::new(false));
        let sig_flag = flag.clone();
        let sig_task = tokio::spawn(async move {
            wait_for_shutdown(sig_flag).await;
            Ok(Action::Terminate)
        });
        let term_flag = flag.clone();
        let term_task = tokio::spawn(async move {
            wait_for_shutdown(term_flag).await;
            Ok(Action::Terminate)
        });
        let main_flag = flag.clone();
        let mut main_task = tokio::spawn(listen(sig_task, term_task, main_flag));

        // Ensure the main task isn't already complete, the first poll gets us to the `.await`
        // and the second poll ensures that it's still pending.
        let mut ctx = dummy_ctx();
        assert!(main_task.poll_unpin(&mut ctx).is_pending());
        assert!(main_task.poll_unpin(&mut ctx).is_pending());

        // Now set the shutdown flag and the task should complete immediately
        flag.store(true, Ordering::SeqCst);
        let action = main_task.await.unwrap();
        assert_eq!(action, Action::Terminate);
    }

    #[tokio::test]
    async fn action_terminate_when_receiving_terminate_signal() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let deliver_signal = Arc::new(AtomicBool::new(true));
        for sig in [SIGINT, SIGTERM, SIGQUIT].into_iter() {
            let mock_listener =
                SignalListener::Mock(mock::MockListener::new(Some(sig), deliver_signal.clone()));
            let sig_task = spawn_signal_listener(mock_listener, shutdown.clone()).unwrap();
            assert_eq!(sig_task.await.unwrap().unwrap(), Action::Terminate);
        }
    }

    #[tokio::test]
    async fn action_cleanup_when_receiving_cleanup_signal() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let deliver_signal = Arc::new(AtomicBool::new(true));
        let mock_listener = SignalListener::Mock(mock::MockListener::new(
            Some(SIGUSR1),
            deliver_signal.clone(),
        ));
        let sig_task = spawn_signal_listener(mock_listener, shutdown.clone()).unwrap();
        assert_eq!(sig_task.await.unwrap().unwrap(), Action::Cleanup);
    }

    #[tokio::test]
    async fn waits_for_termination() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let term_flag = Arc::new(AtomicBool::new(false));
        let term_wait = term_flag.clone();
        let term_task = tokio::spawn(async move {
            // This just blocks until `term_flag` is set
            wait_for_shutdown(term_wait).await;
            Ok(Action::Cleanup)
        });
        let sig_flag = shutdown.clone();
        let sig_task = tokio::spawn(async {
            // This flag is set in `listen` when a termination is detected
            wait_for_shutdown(sig_flag).await;
            Ok(Action::Terminate)
        });
        let main_flag = shutdown.clone();
        let mut main_task = tokio::spawn(listen(sig_task, term_task, main_flag));

        // Ensure the main task isn't already complete, the first poll gets us to the `.await`
        // and the second poll ensures that it's still pending.
        let mut ctx = dummy_ctx();
        assert!(main_task.poll_unpin(&mut ctx).is_pending());
        assert!(main_task.poll_unpin(&mut ctx).is_pending());

        term_flag.store(true, Ordering::SeqCst);
        let action = main_task.await.unwrap();
        assert_eq!(action, Action::Cleanup);
    }
}
