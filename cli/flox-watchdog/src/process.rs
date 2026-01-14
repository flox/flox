//! This module uses platform specific mechanisms to determine when processes
//! are runnable, zombies, or terminated.
//!
//! On Linux we read `/proc`. See the
//! [man page](https://man7.org/linux/man-pages/man5/proc_pid_stat.5.html) for
//! more details.
//!
//! On macOS we slum it and call `/bin/ps` rather than using the private `libproc.h`
//! API, but mostly for build-complexity reasons.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use flox_core::activations::{ActivationState, read_activations_json, write_activations_json};
use flox_core::proc_status::pid_is_running;
use fslock::LockFile;
use signal_hook::iterator::Signals;
use time::OffsetDateTime;
use tracing::trace;

use crate::reaper::reap_orphaned_children;
/// How long to wait between watcher updates.
pub const WATCHER_SLEEP_INTERVAL: Duration = Duration::from_millis(100);

type Error = anyhow::Error;

/// A deserialized activations.json together with a lock preventing it from
/// being modified
/// TODO: there's probably a cleaner way to do this
pub type LockedActivationState = (ActivationState, LockFile);

#[derive(Debug)]
pub enum WaitResult {
    CleanUp(LockedActivationState),
    Terminate,
}

pub trait Watcher {
    /// Block while the watcher waits for a termination or cleanup event.
    fn wait_for_termination(&mut self) -> Result<WaitResult, Error>;
    /// Instructs the watcher to update the list of PIDs that it's watching
    /// by reading the environment registry (for now).
    fn cleanup_pids(&mut self) -> Result<Option<WaitResult>, Error>;
    /// Writes the current activation PIDs back out to `activations.json`
    /// while holding a lock on it.
    fn update_activations_file(
        &self,
        activations: ActivationState,
        lock: LockFile,
    ) -> Result<(), Error>;
}

#[derive(Debug)]
pub struct PidWatcher {
    state_json_path: PathBuf,
    dot_flox_path: PathBuf,
    runtime_dir: PathBuf,
    should_terminate_flag: Arc<AtomicBool>,
    should_clean_up_flag: Arc<AtomicBool>,
    should_reap_signals: Signals,
}

impl PidWatcher {
    /// Creates a new watcher that uses platform-specific mechanisms to wait
    /// for activation processes to terminate.
    pub fn new(
        state_json_path: PathBuf,
        dot_flox_path: PathBuf,
        runtime_dir: PathBuf,
        should_terminate_flag: Arc<AtomicBool>,
        should_clean_up_flag: Arc<AtomicBool>,
        should_reap_signals: Signals,
    ) -> Self {
        Self {
            state_json_path,
            dot_flox_path,
            runtime_dir,
            should_terminate_flag,
            should_clean_up_flag,
            should_reap_signals,
        }
    }
}

impl Watcher for PidWatcher {
    fn wait_for_termination(&mut self) -> Result<WaitResult, Error> {
        loop {
            if let Some(exit) = self.cleanup_pids()? {
                return Ok(exit);
            }
            if self
                .should_terminate_flag
                .load(std::sync::atomic::Ordering::SeqCst)
            {
                return Ok(WaitResult::Terminate);
            }
            if self
                .should_clean_up_flag
                .load(std::sync::atomic::Ordering::SeqCst)
            {
                let (activations_json, lock) = read_activations_json(&self.state_json_path)?;
                let Some(activations_json) = activations_json else {
                    bail!("watchdog shouldn't be running when activations.json doesn't exist");
                };
                return Ok(WaitResult::CleanUp((activations_json, lock)));
            }
            for _ in self.should_reap_signals.pending() {
                reap_orphaned_children();
            }
            std::thread::sleep(WATCHER_SLEEP_INTERVAL);
        }
    }

    /// Reload and check the list of PIDs for an activation.
    fn cleanup_pids(&mut self) -> Result<Option<WaitResult>, Error> {
        let (activations_json, lock) = read_activations_json(&self.state_json_path)?;
        let Some(mut activations) = activations_json else {
            bail!("watchdog shouldn't be running when activations.json doesn't exist");
        };

        let now = OffsetDateTime::now_utc();
        let (empty_start_ids, modified) = activations.cleanup_pids(pid_is_running, now);

        // If there are no more attached PIDs for any start, return early and
        // cleanup the entirety of the activation state directory
        if activations.attached_pids_is_empty() {
            let res = WaitResult::CleanUp((activations, lock));
            return Ok(Some(res));
        }

        // Cleanup empty start IDs
        //
        // We might want to skip this if start_id is the same as that in ready,
        // since otherwise we'll do another start of the same environment when:
        // 1. There were still some activations of the environment
        // and
        // 2. The environment was not modified
        // But I think for now it's simpler to just treat all start_ids the same.
        for start_id in empty_start_ids {
            let state_dir = start_id.state_dir_path(&self.runtime_dir, &self.dot_flox_path)?;
            trace!(?state_dir, "removing empty activation state dir");
            std::fs::remove_dir_all(state_dir).context("failed to remove start state dir")?;
        }

        if modified {
            trace!(?activations, "writing PID changes to activation");
            self.update_activations_file(activations, lock)?;
        }

        Ok(None)
    }

    /// Update the `activations.json` file with the current list of running PIDs.
    fn update_activations_file(
        &self,
        activations: ActivationState,
        lock: LockFile,
    ) -> Result<(), Error> {
        write_activations_json(&activations, &self.state_json_path, lock)
    }
}

#[cfg(test)]
pub mod test {
    use std::collections::BTreeMap;
    use std::process::{Child, Command};
    use std::sync::atomic::Ordering;

    use flox_core::activate::mode::ActivateMode;
    use flox_core::activations::test_helpers::{read_activation_state, write_activation_state};
    use flox_core::activations::{StartOrAttachResult, state_json_path};
    use flox_core::proc_status::{ProcStatus, pid_is_running, read_pid_status};

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

    /// Makes shutdown flags to mimic those used by the watchdog
    pub fn shutdown_flags() -> (Arc<AtomicBool>, Arc<AtomicBool>, Signals) {
        const NO_SIGNALS: &[i32] = &[];
        (
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicBool::new(false)),
            Signals::new(NO_SIGNALS).expect("failed to create Signals"),
        )
    }

    /// Wait some attempts for the process to reach the desired state
    fn poll_until_state(state: ProcStatus, pid: i32) {
        for _ in 0..10 {
            if read_pid_status(pid) == state {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("never entered zombie state");
    }

    #[test]
    fn reports_that_pid1_is_running() {
        assert!(pid_is_running(1));
    }

    #[test]
    fn detects_running_or_not_running_process() {
        let proc = start_process();
        let pid = proc.id() as i32;
        assert!(pid_is_running(pid));
        stop_process(proc);
        assert!(!pid_is_running(pid));
    }

    #[test]
    fn detects_zombie() {
        let mut proc = Command::new("true").spawn().unwrap();
        let pid = proc.id() as i32;
        poll_until_state(ProcStatus::Zombie, pid);
        assert!(!pid_is_running(pid));
        assert_eq!(read_pid_status(pid), ProcStatus::Zombie);
        proc.wait().unwrap();
    }

    #[test]
    fn terminates_when_all_pids_terminate() {
        let runtime_dir = tempfile::tempdir().unwrap();
        let dot_flox_path = PathBuf::from(".flox");
        let flox_env = dot_flox_path.join("run/test");
        let store_path = "store_path".to_string();

        let proc1 = start_process();
        let pid1 = proc1.id() as i32;
        let proc2 = start_process();
        let pid2 = proc2.id() as i32;

        // Create an ActivationState with two PIDs attached to the same start_id
        let mut state = ActivationState::new(&ActivateMode::default(), &dot_flox_path, &flox_env);
        let result = state.start_or_attach(pid1, &store_path);
        let StartOrAttachResult::Start { start_id, .. } = result else {
            panic!("Expected Start")
        };
        state.set_ready(&start_id);
        let result = state.start_or_attach(pid2, &store_path);
        assert!(matches!(result, StartOrAttachResult::Attach { .. }));

        write_activation_state(runtime_dir.path(), &dot_flox_path, state);

        let state_json_path =
            flox_core::activations::state_json_path(runtime_dir.path(), &dot_flox_path);
        let (terminate_flag, cleanup_flag, reap_flag) = shutdown_flags();
        let mut watcher = PidWatcher::new(
            state_json_path,
            dot_flox_path.clone(),
            runtime_dir.path().to_path_buf(),
            terminate_flag,
            cleanup_flag,
            reap_flag,
        );
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let wait_result = std::thread::scope(move |s| {
            let b_clone = barrier.clone();
            let procs_handle = s.spawn(move || {
                b_clone.wait();
                stop_process(proc1);
                stop_process(proc2);
            });
            barrier.wait();
            let watcher_handle = s.spawn(move || watcher.wait_for_termination().unwrap());
            let wait_result = watcher_handle.join().unwrap();
            let _ = procs_handle.join(); // should already have terminated
            wait_result
        });
        assert!(matches!(wait_result, WaitResult::CleanUp(_)));
    }

    /// When an attachment to a start exits, its PID is removed if its the first
    /// PID in the list
    #[test]
    fn terminated_pid_removed_if_first() {
        let runtime_dir = tempfile::tempdir().unwrap();
        let dot_flox_path = PathBuf::from(".flox");
        let flox_env = dot_flox_path.join("run/test");
        let store_path = "store_path".to_string();

        // Start and set ready pid1
        let proc1 = start_process();
        let pid1 = proc1.id() as i32;
        let mut state = ActivationState::new(&ActivateMode::default(), &dot_flox_path, &flox_env);
        let result = state.start_or_attach(pid1, &store_path);
        let StartOrAttachResult::Start { start_id, .. } = result else {
            panic!("Expected Start")
        };

        state.set_ready(&start_id);
        let proc2 = start_process();
        let pid2 = proc2.id() as i32;
        state.start_or_attach(pid2, &store_path);

        // Verify both PIDs are initially present
        let initial_pids = state.attached_pids_running();
        assert_eq!(
            initial_pids,
            vec![pid1, pid2],
            "both pids should be running and present"
        );

        write_activation_state(runtime_dir.path(), &dot_flox_path, state);

        // The cleanup logic is order sensitive,
        // so proc2 wouldn't be cleaned up if we stopped it instead of proc1
        stop_process(proc1);

        let state_json_path = state_json_path(runtime_dir.path(), &dot_flox_path);
        let (terminate_flag, cleanup_flag, reap_flag) = shutdown_flags();
        let mut watcher = PidWatcher::new(
            state_json_path,
            dot_flox_path.clone(),
            runtime_dir.path().to_path_buf(),
            terminate_flag.clone(),
            cleanup_flag,
            reap_flag,
        );

        std::thread::scope(move |s| {
            let watcher_thread = s.spawn(move || watcher.wait_for_termination().unwrap());
            // This wait is just to let the watcher update its watchlist
            // and realize that one of the processes has exited.
            std::thread::sleep(2 * WATCHER_SLEEP_INTERVAL);

            // Check state while proc2 is still running
            let intermediate_state = read_activation_state(runtime_dir.path(), &dot_flox_path);
            let intermediate_attachments = intermediate_state.attachments_by_start_id();
            assert_eq!(
                BTreeMap::from([(start_id.clone(), vec![(pid2, None)])]),
                intermediate_attachments,
                "only pid2 should be running and present after proc1 terminated"
            );

            // Clean up all extra processes and watcher
            stop_process(proc2);
            watcher_thread
                .join()
                .expect("watcher thread didn't exit cleanly");
        });
    }

    /// After all attachments to a start exit, start state directory is removed
    #[test]
    fn cleans_up_start_state_directory() {
        let runtime_dir = tempfile::tempdir().unwrap();
        let dot_flox_path = PathBuf::from(".flox");
        let flox_env = dot_flox_path.join("run/test");
        let store_path_1 = "store_path_1".to_string();
        let store_path_2 = "store_path_2".to_string();

        let proc1 = start_process();
        let pid1 = proc1.id() as i32;
        let proc2 = start_process();
        let pid2 = proc2.id() as i32;

        // Start and set ready for store_path_1
        let mut state = ActivationState::new(&ActivateMode::default(), &dot_flox_path, &flox_env);
        let result = state.start_or_attach(pid1, &store_path_1);
        let StartOrAttachResult::Start {
            start_id: start_id_1,
            ..
        } = result
        else {
            panic!("Expected Start")
        };
        state.set_ready(&start_id_1);

        // Start and set ready for store_path_2
        let result = state.start_or_attach(pid2, &store_path_2);
        let StartOrAttachResult::Start {
            start_id: start_id_2,
            ..
        } = result
        else {
            panic!("Expected Start")
        };
        state.set_ready(&start_id_2);

        write_activation_state(runtime_dir.path(), &dot_flox_path, state);

        // Create both state directories
        let state_dir_1 = start_id_1
            .state_dir_path(runtime_dir.path(), &dot_flox_path)
            .unwrap();
        let state_dir_2 = start_id_2
            .state_dir_path(runtime_dir.path(), &dot_flox_path)
            .unwrap();
        std::fs::create_dir_all(&state_dir_1).unwrap();
        std::fs::create_dir_all(&state_dir_2).unwrap();
        assert!(state_dir_1.exists());
        assert!(state_dir_2.exists());

        let state_json_path = state_json_path(runtime_dir.path(), &dot_flox_path);
        let (terminate_flag, cleanup_flag, reap_flag) = shutdown_flags();
        let mut watcher = PidWatcher::new(
            state_json_path,
            dot_flox_path.clone(),
            runtime_dir.path().to_path_buf(),
            terminate_flag.clone(),
            cleanup_flag,
            reap_flag,
        );

        std::thread::scope(|s| {
            let watcher_thread = s.spawn(move || watcher.wait_for_termination().unwrap());

            stop_process(proc1);

            // Wait for watcher to process the termination
            std::thread::sleep(2 * WATCHER_SLEEP_INTERVAL);

            // Verify state_dir_1 has been removed but state_dir_2 still exists
            assert!(!state_dir_1.exists(), "state directory 1 should be removed");
            assert!(state_dir_2.exists(), "state directory 2 should still exist");

            // Clean up all extra processes and watcher
            terminate_flag.store(true, Ordering::SeqCst);
            stop_process(proc2);
            watcher_thread
                .join()
                .expect("watcher thread didn't exit cleanly");
        });
    }

    #[test]
    fn terminates_on_shutdown_flag() {
        let runtime_dir = tempfile::tempdir().unwrap();
        let dot_flox_path = PathBuf::from(".flox");
        let flox_env = dot_flox_path.join("run/test");
        let store_path = "store_path".to_string();

        let proc = start_process();
        let pid = proc.id() as i32;

        // Create an ActivationState with one PID attached
        let mut state = ActivationState::new(&ActivateMode::default(), &dot_flox_path, &flox_env);
        let result = state.start_or_attach(pid, &store_path);
        let StartOrAttachResult::Start { start_id, .. } = result else {
            panic!("Expected Start")
        };
        state.set_ready(&start_id);

        write_activation_state(runtime_dir.path(), &dot_flox_path, state);

        let state_json_path = state_json_path(runtime_dir.path(), &dot_flox_path);
        let (terminate_flag, cleanup_flag, reap_flag) = shutdown_flags();
        let mut watcher = PidWatcher::new(
            state_json_path,
            dot_flox_path.clone(),
            runtime_dir.path().to_path_buf(),
            terminate_flag.clone(),
            cleanup_flag.clone(),
            reap_flag,
        );
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let wait_result = std::thread::scope(move |s| {
            let b_clone = barrier.clone();
            let flag_handle = s.spawn(move || {
                b_clone.wait();
                terminate_flag.store(true, std::sync::atomic::Ordering::SeqCst);
            });
            barrier.wait();
            let watcher_handle = s.spawn(move || watcher.wait_for_termination().unwrap());
            let wait_result = watcher_handle.join().unwrap();
            let _ = flag_handle.join(); // should already have terminated
            wait_result
        });
        stop_process(proc);
        assert!(matches!(wait_result, WaitResult::Terminate));
    }

    #[test]
    fn terminates_on_signal_handler_flag() {
        let runtime_dir = tempfile::tempdir().unwrap();
        let dot_flox_path = PathBuf::from(".flox");
        let flox_env = dot_flox_path.join("run/test");
        let store_path = "store_path".to_string();

        let proc = start_process();
        let pid = proc.id() as i32;

        // Create an ActivationState with one PID attached
        let mut state = ActivationState::new(&ActivateMode::default(), &dot_flox_path, &flox_env);
        let result = state.start_or_attach(pid, &store_path);
        let StartOrAttachResult::Start { start_id, .. } = result else {
            panic!("Expected Start")
        };
        state.set_ready(&start_id);
        write_activation_state(runtime_dir.path(), &dot_flox_path, state);

        let state_json_path = state_json_path(runtime_dir.path(), &dot_flox_path);
        let (terminate_flag, cleanup_flag, reap_flag) = shutdown_flags();
        let mut watcher = PidWatcher::new(
            state_json_path,
            dot_flox_path.clone(),
            runtime_dir.path().to_path_buf(),
            terminate_flag.clone(),
            cleanup_flag.clone(),
            reap_flag,
        );
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let wait_result = std::thread::scope(move |s| {
            let b_clone = barrier.clone();
            let flag_handle = s.spawn(move || {
                b_clone.wait();
                cleanup_flag.store(true, std::sync::atomic::Ordering::SeqCst);
            });
            barrier.wait();
            let watcher_handle = s.spawn(move || watcher.wait_for_termination().unwrap());
            let wait_result = watcher_handle.join().unwrap();
            let _ = flag_handle.join(); // should already have terminated
            wait_result
        });
        stop_process(proc);
        assert!(matches!(wait_result, WaitResult::CleanUp(_)));
    }
}
