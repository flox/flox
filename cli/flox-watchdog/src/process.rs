//! This module uses platform specific mechanisms to determine when processes
//! are runnable, zombies, or terminated.
//!
//! On Linux we read `/proc`. See the
//! [man page](https://man7.org/linux/man-pages/man5/proc_pid_stat.5.html) for
//! more details.
//!
//! On macOS we slum it and call `/bin/ps` rather than using the private `libproc.h`
//! API, but mostly for build-complexity reasons.

use std::collections::HashSet;
use std::fs::read_to_string;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Result};
use flox_core::activations::{read_activations_json, Activations, AttachedPid, UncheckedVersion};
use fslock::LockFile;
use time::OffsetDateTime;
use tracing::{debug, trace, warn};
/// How long to wait between watcher updates.
pub const WATCHER_SLEEP_INTERVAL: Duration = Duration::from_millis(100);

type Error = anyhow::Error;

/// A deserialized activations.json together with a lock preventing it from
/// being modified
/// TODO: there's probably a cleaner way to do this
pub type LockedActivations = (Activations<UncheckedVersion>, LockFile);

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
    fn update_watchlist(&mut self, hold_lock: bool) -> Result<Option<LockedActivations>, Error>;
    /// Returns true if the watcher determines that it's time to perform
    /// cleanup.
    fn should_clean_up(&self) -> Result<bool, Error>;
}

/// The state that a process is in.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum ProcStatus {
    /// The process is running (or runnable, which includes "idle").
    Running,
    /// The process has exited, but has not been cleaned up by the parent.
    Zombie,
    /// Process is dead and will transition to a zombie or disappear.
    /// Technically we shouldn't see this, but just in case:
    /// https://unix.stackexchange.com/a/653370
    AboutToBeZombie,
    /// The process has terminated and been cleaned up. This is also the fallback
    /// for when there is an error reading the process status.
    Dead,
}

#[derive(Debug)]
pub struct PidWatcher {
    pids_watching: HashSet<AttachedPid>,
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
            pids_watching: HashSet::new(),
            activations_json_path,
            activation_id,
            should_terminate_flag,
            should_clean_up_flag,
        }
    }

    /// Reads the state of a process on macOS using `/bin/ps`, which can report
    /// whether a process is a zombie. This is a stopgap until we someday use
    /// `libproc`. Any failure is interpreted as an indication that the process
    /// is no longer running.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    fn read_pid_status_macos(pid: i32) -> ProcStatus {
        let stdout = match Command::new("/bin/ps")
            .args(["-o", "state=", "-p"])
            .arg(format!("{pid}"))
            .output()
        {
            Ok(output) => output.stdout,
            Err(err) => {
                warn!(
                    %err,
                    pid,
                    "failed while calling /bin/ps, treating as not running"
                );
                return ProcStatus::Dead;
            },
        };
        if let Some(state) = stdout.first() {
            match state {
                // '?' means "unknown" from `ps` included with macOS. Note that
                // this is *not the same* as `procps` on Linux or from Nixpkgs.
                b'Z' | b'?' => ProcStatus::Zombie,
                _ => ProcStatus::Running,
            }
        } else {
            debug!(pid, "no output from /bin/ps, treating as not running");
            ProcStatus::Dead
        }
    }

    /// Tries to read the state of a process on Linux via `/proc`. Any failure
    /// is interpreted as an indication that the process is no longer running.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    fn read_pid_status_linux(pid: i32) -> ProcStatus {
        let path = format!("/proc/{pid}/stat");
        let stat = match read_to_string(path) {
            Ok(stat) => stat,
            Err(err) => {
                trace!(
                    %err,
                    pid,
                    "failed to parse /proc/<pid>/stat, treating as not running"
                );
                return ProcStatus::Dead;
            },
        };
        // `/proc/{pid}/stat` has space separated values `pid comm state ...`
        // and we need to extract state
        if let Some(state) = stat
            .split_whitespace()
            .nth(2)
            .and_then(|chars| chars.as_bytes().first())
        {
            match state {
                b'X' | b'x' => ProcStatus::AboutToBeZombie,
                b'Z' => ProcStatus::Zombie,
                _ => ProcStatus::Running,
            }
        } else {
            warn!(
                pid,
                "failed to parse /proc/<pid>/stat, treating as not running"
            );
            ProcStatus::Dead
        }
    }

    /// Returns the status of the provided PID.
    fn read_pid_status(pid: i32) -> ProcStatus {
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        panic!("unsupported operating system");

        #[cfg(target_os = "linux")]
        let status = Self::read_pid_status_linux(pid);

        #[cfg(target_os = "macos")]
        let status = Self::read_pid_status_macos(pid);

        status
    }

    /// Returns whether the process is considered running.
    pub fn pid_is_running(pid: i32) -> bool {
        Self::read_pid_status(pid) == ProcStatus::Running
    }

    /// Removes any PIDs that are no longer running from the watchlist.
    fn prune_terminations(&mut self) {
        let now = OffsetDateTime::now_utc();
        self.pids_watching.retain(|attached_pid| {
            if let Some(expiration) = attached_pid.expiration {
                // If the PID has an unreached expiration, retain it even if it
                // isn't running
                now < expiration || Self::pid_is_running(attached_pid.pid)
            } else {
                Self::pid_is_running(attached_pid.pid)
            }
        })
    }
}

impl Watcher for PidWatcher {
    fn wait_for_termination(&mut self) -> Result<WaitResult, Error> {
        loop {
            self.update_watchlist(false)?;
            if self.should_clean_up()? {
                // Don't hold the lock during normal polling to avoid contention
                // But when we're actually ready to cleanup, we need to hold the lock
                // TODO: could probably refactor and get rid of the unwrap
                let locked_activations = self.update_watchlist(true)?.ok_or(anyhow::anyhow!(
                    "update_watchlist always returns Some when hold_lock is true"
                ))?;
                if self.should_clean_up()? {
                    return Ok(WaitResult::CleanUp(locked_activations));
                };
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
                return Ok(WaitResult::CleanUp((activations_json, lock)));
            }
            std::thread::sleep(WATCHER_SLEEP_INTERVAL);
        }
    }

    /// Update the list of PIDs that are currently being watched.
    fn update_watchlist(&mut self, hold_lock: bool) -> Result<Option<LockedActivations>, Error> {
        let (activations_json, lock) = read_activations_json(&self.activations_json_path)?;
        let Some(activations_json) = activations_json else {
            bail!("watchdog shouldn't be running when activations.json doesn't exist");
        };
        let maybe_locked_activations = if hold_lock {
            Some((activations_json.clone(), lock))
        } else {
            drop(lock);
            None
        };

        let Some(activation) = activations_json.activation_for_id_ref(&self.activation_id) else {
            bail!("watchdog shouldn't be running with ID that isn't in activations.json");
        };

        let all_attached_pids: HashSet<AttachedPid> = activation
            .attached_pids()
            .iter()
            .map(AttachedPid::to_owned)
            .collect();
        // Add all PIDs, even if they're dead, but then immediately remove them
        self.pids_watching.extend(all_attached_pids);
        self.prune_terminations();
        Ok(maybe_locked_activations)
    }

    /// Returns true if the watcher is not currently watching any PIDs.
    fn should_clean_up(&self) -> Result<bool, super::Error> {
        let should_clean_up = self.pids_watching.is_empty();
        if !should_clean_up {
            trace!("still watching PIDs {:?}", self.pids_watching);
        }
        Ok(should_clean_up)
    }
}

#[cfg(test)]
pub mod test {
    use std::path::PathBuf;
    use std::process::{Child, Command};
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    use flox_activations::cli::attach::{AttachArgs, AttachExclusiveArgs};
    use flox_activations::cli::{SetReadyArgs, StartOrAttachArgs};
    use flox_core::activations::activations_json_path;

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
            if PidWatcher::read_pid_status(pid) == state {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("never entered zombie state");
    }

    #[test]
    fn reports_that_pid1_is_running() {
        assert!(PidWatcher::pid_is_running(1));
    }

    #[test]
    fn detects_running_or_not_running_process() {
        let proc = start_process();
        let pid = proc.id() as i32;
        assert!(PidWatcher::pid_is_running(pid));
        stop_process(proc);
        assert!(!PidWatcher::pid_is_running(pid));
    }

    #[test]
    fn detects_zombie() {
        let mut proc = Command::new("true").spawn().unwrap();
        let pid = proc.id() as i32;
        poll_until_state(ProcStatus::Zombie, pid);
        assert!(!PidWatcher::pid_is_running(pid));
        assert_eq!(PidWatcher::read_pid_status(pid), ProcStatus::Zombie);
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
        };
        let activation_id = start_or_attach_pid1.handle(runtime_dir.path()).unwrap();
        let set_ready_pid1 = SetReadyArgs {
            id: activation_id.clone(),
            flox_env: flox_env.clone(),
        };
        set_ready_pid1.handle(runtime_dir.path()).unwrap();

        let proc2 = start_process();
        let pid2 = proc2.id() as i32;
        let start_or_attach_pid2 = StartOrAttachArgs {
            pid: pid2,
            flox_env: flox_env.clone(),
            store_path: store_path.clone(),
        };
        let activation_id_2 = start_or_attach_pid2.handle(runtime_dir.path()).unwrap();
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
    fn pid_not_pruned_before_expiration() {
        let runtime_dir = tempfile::tempdir().unwrap();
        let flox_env = PathBuf::from("flox_env");
        let store_path = "store_path".to_string();

        let proc1 = start_process();
        let pid1 = proc1.id() as i32;
        let start_or_attach_pid1 = StartOrAttachArgs {
            pid: pid1,
            flox_env: flox_env.clone(),
            store_path: store_path.clone(),
        };
        let activation_id = start_or_attach_pid1.handle(runtime_dir.path()).unwrap();
        let set_ready_pid1 = SetReadyArgs {
            id: activation_id.clone(),
            flox_env: flox_env.clone(),
        };
        set_ready_pid1.handle(runtime_dir.path()).unwrap();

        let proc2 = start_process();
        let pid2 = proc2.id() as i32;
        let start_or_attach_pid2 = StartOrAttachArgs {
            pid: pid2,
            flox_env: flox_env.clone(),
            store_path: store_path.clone(),
        };
        let activation_id_2 = start_or_attach_pid2.handle(runtime_dir.path()).unwrap();
        assert_eq!(activation_id, activation_id_2);

        let proc3 = start_process();
        let pid3 = proc3.id() as i32;
        let timeout_ms = 9999;
        let attach_pid3 = AttachArgs {
            flox_env: flox_env.clone(),
            id: activation_id.clone(),
            pid: pid3,
            exclusive: AttachExclusiveArgs {
                timeout_ms: Some(timeout_ms),
                remove_pid: None,
            },
        };
        let now = OffsetDateTime::now_utc();
        let expiration = Some(now + Duration::from_millis(timeout_ms as u64));
        attach_pid3.handle_inner(runtime_dir.path(), now).unwrap();

        let activations_json_path = activations_json_path(&runtime_dir, &flox_env);
        let (terminate_flag, cleanup_flag) = shutdown_flags();
        let mut watcher = PidWatcher::new(
            activations_json_path,
            activation_id,
            terminate_flag,
            cleanup_flag,
        );
        watcher.update_watchlist(false).unwrap();

        assert_eq!(
            watcher.pids_watching,
            HashSet::from([
                AttachedPid {
                    pid: pid1,
                    expiration: None,
                },
                AttachedPid {
                    pid: pid2,
                    expiration: None,
                },
                AttachedPid {
                    pid: pid3,
                    expiration,
                }
            ])
        );

        stop_process(proc1);
        stop_process(proc2);
        stop_process(proc3);

        watcher.update_watchlist(false).unwrap();

        assert!(!watcher.should_clean_up().unwrap());
        assert_eq!(
            watcher.pids_watching,
            HashSet::from([AttachedPid {
                pid: pid3,
                expiration,
            }])
        );
    }

    #[test]
    fn pid_pruned_after_expiration() {
        let runtime_dir = tempfile::tempdir().unwrap();
        let flox_env = PathBuf::from("flox_env");
        let store_path = "store_path".to_string();

        let proc1 = start_process();
        let pid1 = proc1.id() as i32;
        let start_or_attach_pid1 = StartOrAttachArgs {
            pid: pid1,
            flox_env: flox_env.clone(),
            store_path: store_path.clone(),
        };
        let activation_id = start_or_attach_pid1.handle(runtime_dir.path()).unwrap();
        let set_ready_pid1 = SetReadyArgs {
            id: activation_id.clone(),
            flox_env: flox_env.clone(),
        };
        set_ready_pid1.handle(runtime_dir.path()).unwrap();

        let proc2 = start_process();
        let pid2 = proc2.id() as i32;
        let start_or_attach_pid2 = StartOrAttachArgs {
            pid: pid2,
            flox_env: flox_env.clone(),
            store_path: store_path.clone(),
        };
        let activation_id_2 = start_or_attach_pid2.handle(runtime_dir.path()).unwrap();
        assert_eq!(activation_id, activation_id_2);

        let proc3 = start_process();
        let pid3 = proc3.id() as i32;
        let attach_pid3 = AttachArgs {
            flox_env: flox_env.clone(),
            id: activation_id.clone(),
            pid: pid3,
            exclusive: AttachExclusiveArgs {
                timeout_ms: Some(0),
                remove_pid: None,
            },
        };
        let the_past = OffsetDateTime::now_utc() - Duration::from_secs(9999);
        attach_pid3
            .handle_inner(runtime_dir.path(), the_past)
            .unwrap();

        let activations_json_path = activations_json_path(&runtime_dir, &flox_env);
        let (terminate_flag, cleanup_flag) = shutdown_flags();
        let mut watcher = PidWatcher::new(
            activations_json_path,
            activation_id,
            terminate_flag,
            cleanup_flag,
        );
        watcher.update_watchlist(false).unwrap();

        assert_eq!(
            watcher.pids_watching,
            HashSet::from([
                AttachedPid {
                    pid: pid1,
                    expiration: None,
                },
                AttachedPid {
                    pid: pid2,
                    expiration: None,
                },
                AttachedPid {
                    pid: pid3,
                    expiration: Some(the_past),
                }
            ])
        );

        stop_process(proc1);
        stop_process(proc2);
        stop_process(proc3);

        watcher.update_watchlist(false).unwrap();

        assert!(watcher.should_clean_up().unwrap());
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
        };
        let activation_id = start_or_attach.handle(runtime_dir.path()).unwrap();
        let set_ready = SetReadyArgs {
            id: activation_id.clone(),
            flox_env: flox_env.clone(),
        };
        set_ready.handle(runtime_dir.path()).unwrap();

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
        };
        let activation_id = start_or_attach.handle(runtime_dir.path()).unwrap();
        let set_ready = SetReadyArgs {
            id: activation_id.clone(),
            flox_env: flox_env.clone(),
        };
        set_ready.handle(runtime_dir.path()).unwrap();

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
