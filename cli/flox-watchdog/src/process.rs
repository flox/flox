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
use flox_core::activations::rewrite::{
    ActivationState,
    read_activations_json,
    write_activations_json,
};
use flox_core::proc_status::pid_is_running;
use fslock::LockFile;
use signal_hook::iterator::Signals;
use tracing::trace;

use crate::reaper::reap_orphaned_children;
/// How long to wait between watcher updates.
pub const WATCHER_SLEEP_INTERVAL: Duration = Duration::from_millis(100);

type Error = anyhow::Error;

/// A deserialized activations.json together with a lock preventing it from
/// being modified
/// TODO: there's probably a cleaner way to do this
pub type LockedActivations = (ActivationState, LockFile);

#[derive(Debug)]
pub enum WaitResult {
    CleanUp(LockedActivations),
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

        let mut modified = false;
        let attachments_by_start_id = activations.attached_pids_by_start_id();
        let mut empty_start_ids = Vec::new();

        for (start_id, pids) in attachments_by_start_id {
            let mut all_pids_terminated = true;
            for pid in pids {
                if pid_is_running(pid) {
                    // We can skip checking other start_ids when at least one PID is still running.
                    all_pids_terminated = false;
                    break;
                } else {
                    // PID exited. Detach it.
                    // "Clean up after a StartID after there are no more attachments to that StartID"
                    // We need to detach THIS pid.
                    activations.detach(pid);
                    modified = true;
                }
            }

            if all_pids_terminated {
                empty_start_ids.push(start_id);
            }
        }

        // If there are no more attached PIDs for any start, return early and
        // cleanup the entirety of the activation state directory
        if activations.attached_pids_is_empty() {
            let res = WaitResult::CleanUp((activations, lock));
            return Ok(Some(res));
        }

        // TODO: should this and the above loop be implemented in ActivationState?
        activations.update_ready_after_detach();

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
    use std::path::PathBuf;
    use std::process::{Child, Command};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use flox_activations::cli::{SetReadyArgs, StartOrAttachArgs};
    use flox_core::activations::state_json_path;
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
        let store_path = "store_path".to_string();

        let proc1 = start_process();
        let pid1 = proc1.id() as i32;
        let start_or_attach_pid1 = StartOrAttachArgs {
            pid: pid1,
            dot_flox_path: dot_flox_path.clone(),
            store_path: store_path.clone(),
            runtime_dir: runtime_dir.path().to_path_buf(),
        };
        let activation_id = start_or_attach_pid1.handle_inner().unwrap().activation_id;
        let set_ready_pid1 = SetReadyArgs {
            id: activation_id.clone(),
            dot_flox_path: dot_flox_path.clone(),
            runtime_dir: runtime_dir.path().to_path_buf(),
        };
        set_ready_pid1.handle().unwrap();

        let proc2 = start_process();
        let pid2 = proc2.id() as i32;
        let start_or_attach_pid2 = StartOrAttachArgs {
            pid: pid2,
            dot_flox_path: dot_flox_path.clone(),
            store_path: store_path.clone(),
            runtime_dir: runtime_dir.path().to_path_buf(),
        };
        let activation_id_2 = start_or_attach_pid2.handle_inner().unwrap().activation_id;
        assert_eq!(activation_id, activation_id_2);

        let activations_json_path = state_json_path(&runtime_dir, &dot_flox_path);
        let (terminate_flag, cleanup_flag, reap_flag) = shutdown_flags();
        let mut watcher = PidWatcher::new(
            activations_json_path,
            activation_id,
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

    #[test]
    fn terminated_pids_removed_from_activations_file() {
        let runtime_dir = tempfile::tempdir().unwrap();
        let dot_flox_path = PathBuf::from(".flox");
        let store_path = "store_path".to_string();

        let proc1 = start_process();
        let pid1 = proc1.id() as i32;
        let start_or_attach_pid1 = StartOrAttachArgs {
            pid: pid1,
            dot_flox_path: dot_flox_path.clone(),
            store_path: store_path.clone(),
            runtime_dir: runtime_dir.path().to_path_buf(),
        };
        let activation_id = start_or_attach_pid1.handle_inner().unwrap().activation_id;
        let set_ready_pid1 = SetReadyArgs {
            id: activation_id.clone(),
            dot_flox_path: dot_flox_path.clone(),
            runtime_dir: runtime_dir.path().to_path_buf(),
        };
        set_ready_pid1.handle().unwrap();

        let proc2 = start_process();
        let pid2 = proc2.id() as i32;
        let start_or_attach_pid2 = StartOrAttachArgs {
            pid: pid2,
            dot_flox_path: dot_flox_path.clone(),
            store_path: store_path.clone(),
            runtime_dir: runtime_dir.path().to_path_buf(),
        };
        let activation_id_2 = start_or_attach_pid2.handle_inner().unwrap().activation_id;
        assert_eq!(activation_id, activation_id_2);

        let activations_json_path = state_json_path(&runtime_dir, &dot_flox_path);

        // Grab the existing activations before starting the PidWatcher so we
        // can compare against the state after one of the processes has died.
        let (maybe_initial_activations, lockfile) =
            read_activations_json(&activations_json_path).expect("failed to read activations.json");
        let Some(initial_activations_unchecked) = maybe_initial_activations else {
            panic!("no activations were initially recorded")
        };
        let initial_activations = initial_activations_unchecked.check_version().unwrap();
        let initial_pids = initial_activations
            .activation_for_store_path(&store_path)
            .expect("there was no activation for this store path")
            .attached_pids()
            .iter()
            .map(|pid| pid.pid)
            .collect::<Vec<_>>();
        assert_eq!(
            initial_pids,
            vec![pid1, pid2],
            "both pids should be running and present"
        );
        drop(lockfile); // Prevents a deadlock
        stop_process(proc1);

        let (terminate_flag, cleanup_flag, reap_flag) = shutdown_flags();
        let mut watcher = PidWatcher::new(
            activations_json_path.clone(),
            activation_id,
            terminate_flag.clone(),
            cleanup_flag,
            reap_flag,
        );
        let maybe_final_activations = std::thread::scope(move |s| {
            let watcher_thread = s.spawn(move || watcher.wait_for_termination().unwrap());
            // This wait is just to let the watcher update its watchlist
            // and realize that one of the processes has exited.
            std::thread::sleep(2 * WATCHER_SLEEP_INTERVAL);
            let (activations, lockfile) = read_activations_json(&activations_json_path)
                .expect("failed to read actiations.json");
            drop(lockfile);
            terminate_flag.store(true, Ordering::SeqCst);
            stop_process(proc2);
            watcher_thread
                .join()
                .expect("watcher thread didn't exit cleanly");
            activations
        });
        let Some(final_activations_unchecked) = maybe_final_activations else {
            panic!("no activations found at the end")
        };
        let final_pids = final_activations_unchecked
            .check_version()
            .unwrap()
            .activation_for_store_path(&store_path)
            .expect("there was no activation for this store path")
            .attached_pids()
            .iter()
            .map(|pid| pid.pid)
            .collect::<Vec<_>>();
        assert_eq!(
            final_pids,
            vec![pid2],
            "only pid2 should be running and present"
        );
    }

    #[test]
    fn terminates_on_shutdown_flag() {
        let runtime_dir = tempfile::tempdir().unwrap();
        let dot_flox_path = PathBuf::from(".flox");
        let store_path = "store_path".to_string();

        let proc = start_process();
        let pid = proc.id() as i32;
        let start_or_attach = StartOrAttachArgs {
            pid,
            dot_flox_path: dot_flox_path.clone(),
            store_path: store_path.clone(),
            runtime_dir: runtime_dir.path().to_path_buf(),
        };
        let activation_id = start_or_attach.handle_inner().unwrap().activation_id;
        let set_ready = SetReadyArgs {
            id: activation_id.clone(),
            dot_flox_path: dot_flox_path.clone(),
            runtime_dir: runtime_dir.path().to_path_buf(),
        };
        set_ready.handle().unwrap();

        let activations_json_path = state_json_path(&runtime_dir, &dot_flox_path);
        let (terminate_flag, cleanup_flag, reap_flag) = shutdown_flags();
        let mut watcher = PidWatcher::new(
            activations_json_path,
            activation_id,
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
        let store_path = "store_path".to_string();

        let proc = start_process();
        let pid = proc.id() as i32;
        let start_or_attach = StartOrAttachArgs {
            pid,
            dot_flox_path: dot_flox_path.clone(),
            store_path: store_path.clone(),
            runtime_dir: runtime_dir.path().to_path_buf(),
        };
        let activation_id = start_or_attach.handle_inner().unwrap().activation_id;
        let set_ready = SetReadyArgs {
            id: activation_id.clone(),
            dot_flox_path: dot_flox_path.clone(),
            runtime_dir: runtime_dir.path().to_path_buf(),
        };
        set_ready.handle().unwrap();

        let activations_json_path = state_json_path(&runtime_dir, &dot_flox_path);
        let (terminate_flag, cleanup_flag, reap_flag) = shutdown_flags();
        let mut watcher = PidWatcher::new(
            activations_json_path,
            activation_id,
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
