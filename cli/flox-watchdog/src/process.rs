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

use anyhow::{Result, bail};
use flox_core::activations::{
    Activations,
    CheckedVersion,
    read_activations_json,
    write_activations_json,
};
use fslock::LockFile;
use tracing::trace;
/// How long to wait between watcher updates.
pub const WATCHER_SLEEP_INTERVAL: Duration = Duration::from_millis(100);

type Error = anyhow::Error;

/// A deserialized activations.json together with a lock preventing it from
/// being modified
/// TODO: there's probably a cleaner way to do this
pub type LockedActivations = (Activations<CheckedVersion>, LockFile);

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
    fn check_pids(&mut self) -> Result<Option<WaitResult>, Error>;
    /// Writes the current activation PIDs back out to `activations.json`
    /// while holding a lock on it.
    fn update_activations_file(
        &self,
        activations: Activations<CheckedVersion>,
        lock: LockFile,
    ) -> Result<(), Error>;
}

#[derive(Debug)]
pub struct PidWatcher {
    activation_id: String,
    activations_json_path: PathBuf,
    should_terminate_flag: Arc<AtomicBool>,
    should_clean_up_flag: Arc<AtomicBool>,
}

impl PidWatcher {
    /// Creates a new watcher that uses platform-specific mechanisms to wait
    /// for activation processes to terminate.
    pub fn new(
        activations_json_path: PathBuf,
        activation_id: String,
        should_terminate_flag: Arc<AtomicBool>,
        should_clean_up_flag: Arc<AtomicBool>,
    ) -> Self {
        Self {
            activations_json_path,
            activation_id,
            should_terminate_flag,
            should_clean_up_flag,
        }
    }
}

impl Watcher for PidWatcher {
    fn wait_for_termination(&mut self) -> Result<WaitResult, Error> {
        loop {
            if let Some(exit) = self.check_pids()? {
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
                let (activations_json, lock) = read_activations_json(&self.activations_json_path)?;
                let Some(activations_json) = activations_json else {
                    bail!("watchdog shouldn't be running when activations.json doesn't exist");
                };
                let activations = activations_json.check_version()?;
                return Ok(WaitResult::CleanUp((activations, lock)));
            }
            std::thread::sleep(WATCHER_SLEEP_INTERVAL);
        }
    }

    /// Reload and check the list of PIDs for an activation.
    fn check_pids(&mut self) -> Result<Option<WaitResult>, Error> {
        let (activations_json, lock) = read_activations_json(&self.activations_json_path)?;
        let Some(activations_json) = activations_json else {
            bail!("watchdog shouldn't be running when activations.json doesn't exist");
        };

        // NOTE(zmitchell, 2025-07-28): at some point we'll have to handle migrations here
        // if there are updates to the `activations.json` schema.
        let mut activations = activations_json.check_version()?;

        let Some(activation) = activations.activation_for_id_mut(&self.activation_id) else {
            bail!("watchdog shouldn't be running with ID that isn't in activations.json");
        };

        let pids_modified = activation.remove_terminated_pids();
        let pids = activation.attached_pids();
        if pids.is_empty() {
            let res = WaitResult::CleanUp((activations, lock));
            return Ok(Some(res));
        }

        trace!("still watching PIDs {:?}", pids);
        // Only write changes after checking if we need to exit.
        if pids_modified {
            trace!(?activation, "writing PID changes to activation");
            self.update_activations_file(activations, lock)?;
        }

        Ok(None)
    }

    /// Update the `activations.json` file with the current list of running PIDs.
    fn update_activations_file(
        &self,
        activations: Activations<CheckedVersion>,
        lock: LockFile,
    ) -> Result<(), Error> {
        write_activations_json(&activations, &self.activations_json_path, lock)
    }
}

#[cfg(test)]
pub mod test {
    use std::path::PathBuf;
    use std::process::{Child, Command};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use flox_activations::cli::{SetReadyArgs, StartOrAttachArgs};
    use flox_core::activations::activations_json_path;
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

    /// Makes two Arc<AtomicBool>s to mimic the shutdown flags used by
    /// the watchdog
    pub fn shutdown_flags() -> (Arc<AtomicBool>, Arc<AtomicBool>) {
        (
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicBool::new(false)),
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
        let flox_env = PathBuf::from("flox_env");
        let store_path = "store_path".to_string();

        let proc1 = start_process();
        let pid1 = proc1.id() as i32;
        let start_or_attach_pid1 = StartOrAttachArgs {
            pid: pid1,
            flox_env: flox_env.clone(),
            store_path: store_path.clone(),
            runtime_dir: runtime_dir.path().to_path_buf(),
        };
        let activation_id = start_or_attach_pid1.handle_inner().unwrap().activation_id;
        let set_ready_pid1 = SetReadyArgs {
            id: activation_id.clone(),
            flox_env: flox_env.clone(),
            runtime_dir: runtime_dir.path().to_path_buf(),
        };
        set_ready_pid1.handle().unwrap();

        let proc2 = start_process();
        let pid2 = proc2.id() as i32;
        let start_or_attach_pid2 = StartOrAttachArgs {
            pid: pid2,
            flox_env: flox_env.clone(),
            store_path: store_path.clone(),
            runtime_dir: runtime_dir.path().to_path_buf(),
        };
        let activation_id_2 = start_or_attach_pid2.handle_inner().unwrap().activation_id;
        assert_eq!(activation_id, activation_id_2);

        let activations_json_path = activations_json_path(&runtime_dir, &flox_env);
        let (terminate_flag, cleanup_flag) = shutdown_flags();
        let mut watcher = PidWatcher::new(
            activations_json_path,
            activation_id,
            terminate_flag,
            cleanup_flag,
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
        let flox_env = PathBuf::from("flox_env");
        let store_path = "store_path".to_string();

        let proc1 = start_process();
        let pid1 = proc1.id() as i32;
        let start_or_attach_pid1 = StartOrAttachArgs {
            pid: pid1,
            flox_env: flox_env.clone(),
            store_path: store_path.clone(),
            runtime_dir: runtime_dir.path().to_path_buf(),
        };
        let activation_id = start_or_attach_pid1.handle_inner().unwrap().activation_id;
        let set_ready_pid1 = SetReadyArgs {
            id: activation_id.clone(),
            flox_env: flox_env.clone(),
            runtime_dir: runtime_dir.path().to_path_buf(),
        };
        set_ready_pid1.handle().unwrap();

        let proc2 = start_process();
        let pid2 = proc2.id() as i32;
        let start_or_attach_pid2 = StartOrAttachArgs {
            pid: pid2,
            flox_env: flox_env.clone(),
            store_path: store_path.clone(),
            runtime_dir: runtime_dir.path().to_path_buf(),
        };
        let activation_id_2 = start_or_attach_pid2.handle_inner().unwrap().activation_id;
        assert_eq!(activation_id, activation_id_2);

        let activations_json_path = activations_json_path(&runtime_dir, &flox_env);

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

        let (terminate_flag, cleanup_flag) = shutdown_flags();
        let mut watcher = PidWatcher::new(
            activations_json_path.clone(),
            activation_id,
            terminate_flag.clone(),
            cleanup_flag,
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
        let flox_env = PathBuf::from("flox_env");
        let store_path = "store_path".to_string();

        let proc = start_process();
        let pid = proc.id() as i32;
        let start_or_attach = StartOrAttachArgs {
            pid,
            flox_env: flox_env.clone(),
            store_path: store_path.clone(),
            runtime_dir: runtime_dir.path().to_path_buf(),
        };
        let activation_id = start_or_attach.handle_inner().unwrap().activation_id;
        let set_ready = SetReadyArgs {
            id: activation_id.clone(),
            flox_env: flox_env.clone(),
            runtime_dir: runtime_dir.path().to_path_buf(),
        };
        set_ready.handle().unwrap();

        let activations_json_path = activations_json_path(&runtime_dir, &flox_env);
        let (terminate_flag, cleanup_flag) = shutdown_flags();
        let mut watcher = PidWatcher::new(
            activations_json_path,
            activation_id,
            terminate_flag.clone(),
            cleanup_flag.clone(),
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
        let flox_env = PathBuf::from("flox_env");
        let store_path = "store_path".to_string();

        let proc = start_process();
        let pid = proc.id() as i32;
        let start_or_attach = StartOrAttachArgs {
            pid,
            flox_env: flox_env.clone(),
            store_path: store_path.clone(),
            runtime_dir: runtime_dir.path().to_path_buf(),
        };
        let activation_id = start_or_attach.handle_inner().unwrap().activation_id;
        let set_ready = SetReadyArgs {
            id: activation_id.clone(),
            flox_env: flox_env.clone(),
            runtime_dir: runtime_dir.path().to_path_buf(),
        };
        set_ready.handle().unwrap();

        let activations_json_path = activations_json_path(&runtime_dir, &flox_env);
        let (terminate_flag, cleanup_flag) = shutdown_flags();
        let mut watcher = PidWatcher::new(
            activations_json_path,
            activation_id,
            terminate_flag.clone(),
            cleanup_flag.clone(),
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
