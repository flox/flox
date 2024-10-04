use std::collections::HashSet;
use std::fs::read_to_string;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::bail;
use flox_rust_sdk::models::env_registry::{activation_pids, ActivationPid};
use tracing::{debug, warn};

use super::{Error, WaitResult, Watcher, WATCHER_SLEEP_INTERVAL};

/// https://man7.org/linux/man-pages/man5/proc_pid_stat.5.html
/// Process has finished but the parent hasn't reaped it yet.
const PROCFS_STATE_ZOMBIE: &str = "Z";
/// Process is dead and will transition to a zombie or disappear.
/// Technically we shouldn't see this, but just in case:
/// https://unix.stackexchange.com/a/653370
const PROCFS_STATE_DEAD: &str = "X";
const PROCFS_STATE_DEAD_COMPAT: &str = "x";

#[derive(Debug)]
pub struct LinuxWatcher {
    pub original_pid: ActivationPid,
    pub pids_watching: HashSet<ActivationPid>,
    pub reg_path: PathBuf,
    pub hash: String,
    pub should_terminate_flag: Arc<AtomicBool>,
    pub should_clean_up_flag: Arc<AtomicBool>,
}

impl LinuxWatcher {
    /// Creates a new watcher that reads `/proc` to get process status.
    pub fn new(
        pid: ActivationPid,
        reg_path: impl AsRef<Path>,
        hash: impl AsRef<str>,
        should_terminate_flag: Arc<AtomicBool>,
        should_clean_up_flag: Arc<AtomicBool>,
    ) -> Self {
        Self {
            original_pid: pid,
            pids_watching: HashSet::new(),
            reg_path: PathBuf::from(reg_path.as_ref()),
            hash: String::from(hash.as_ref()),
            should_terminate_flag,
            should_clean_up_flag,
        }
    }

    /// Reads the state of a process. Any failure is used as an indication that
    /// the process is no longer running.
    pub(crate) fn try_read_pid_status(pid: ActivationPid) -> Option<String> {
        let path = format!("/proc/{}/stat", pid);
        let pid_raw: i32 = pid.into();
        let stat = match read_to_string(path) {
            Ok(stat) => stat,
            Err(err) => {
                debug!(%err, pid = pid_raw, "failed to read stat, treating as not running");
                return None;
            },
        };
        // `/proc/{pid}/stat` has space separated values `pid comm state ...`
        // and we need to extract state
        let state = match stat.split_whitespace().nth(2) {
            Some(state) => state.to_string(),
            None => {
                warn!(
                    pid = pid_raw,
                    "failed to parse stat, treating as not running"
                );
                return None;
            },
        };
        Some(state)
    }

    /// Returns whether the process is considered running.
    pub(crate) fn pid_is_running(pid: ActivationPid) -> bool {
        Self::try_read_pid_status(pid).is_some_and(|state| {
            !matches!(
                state.as_str(),
                PROCFS_STATE_ZOMBIE | PROCFS_STATE_DEAD | PROCFS_STATE_DEAD_COMPAT
            )
        })
    }

    fn prune_terminations(&mut self) {
        self.pids_watching.retain(|&pid| Self::pid_is_running(pid));
    }
}

impl Watcher for LinuxWatcher {
    fn wait_for_termination(&mut self) -> Result<WaitResult, Error> {
        self.pids_watching.insert(self.original_pid);
        loop {
            self.update_watchlist()?;
            if self.should_clean_up()? {
                return Ok(WaitResult::CleanUp);
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
                return Ok(WaitResult::CleanUp);
            }
            std::thread::sleep(WATCHER_SLEEP_INTERVAL);
        }
    }

    /// Update the list of PIDs that are currently being watched.
    fn update_watchlist(&mut self) -> Result<(), Error> {
        let all_registered_pids = activation_pids(&self.reg_path, &self.hash)?;
        self.prune_terminations();
        let to_add = self
            .pids_watching
            .difference(&all_registered_pids)
            .cloned()
            .collect::<Vec<_>>();
        for pid in to_add {
            if pid.is_running() && self.pids_watching.insert(pid) {
                // Only triggered if the insert reports that the PID was already
                // being watched
                bail!("tried to watch PID {pid}, which was already watched");
            }
        }
        Ok(())
    }

    fn should_clean_up(&self) -> Result<bool, super::Error> {
        Ok(self.pids_watching.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use std::process::Command;
    use std::thread;
    use std::time::Duration;

    use flox_rust_sdk::models::env_registry::register_activation;

    use super::*;
    use crate::process::test::{
        path_for_registry_with_entry,
        shutdown_flags,
        start_process,
        stop_process,
    };

    /// Wait some attempts for the process to reach the desired state
    fn poll_until_state(state: &str, pid: ActivationPid) {
        for _ in 0..10 {
            if let Some(s) = LinuxWatcher::try_read_pid_status(pid) {
                if s == state {
                    break;
                }
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn reports_that_pid1_is_running() {
        assert!(LinuxWatcher::pid_is_running(1.into()));
    }

    #[test]
    fn detects_running_or_not_running_process() {
        let proc = start_process();
        let pid = proc.id() as i32;
        assert!(LinuxWatcher::pid_is_running(pid.into()));
        stop_process(proc);
        assert!(!LinuxWatcher::pid_is_running(pid.into()));
    }

    #[test]
    fn detects_zombie() {
        let mut proc = Command::new("true").spawn().unwrap();
        let pid = proc.id() as i32;
        poll_until_state(PROCFS_STATE_ZOMBIE, pid.into());
        assert!(!LinuxWatcher::pid_is_running(pid.into()));
        assert_eq!(
            LinuxWatcher::try_read_pid_status(pid.into()),
            Some(PROCFS_STATE_ZOMBIE.to_string())
        );
        proc.wait().unwrap();
    }

    #[test]
    fn terminates_when_all_pids_terminate() {
        let proc1 = start_process();
        let pid1 = ActivationPid::from(proc1.id() as i32);
        let proc2 = start_process();
        let (terminate_flag, cleanup_flag) = shutdown_flags();
        let path_hash = "abc";
        let reg_path = path_for_registry_with_entry(&path_hash);
        register_activation(&reg_path, &path_hash, pid1).unwrap();
        let mut watcher =
            LinuxWatcher::new(pid1, &reg_path, &path_hash, terminate_flag, cleanup_flag);
        let wait_result = std::thread::scope(move |s| {
            let procs_handle = s.spawn(|| {
                std::thread::sleep(Duration::from_millis(100));
                stop_process(proc1);
                stop_process(proc2);
            });
            let watcher_handle = s.spawn(move || watcher.wait_for_termination().unwrap());
            let wait_result = watcher_handle.join().unwrap();
            let _ = procs_handle.join(); // should already have terminated
            wait_result
        });
        assert_eq!(wait_result, WaitResult::CleanUp);
    }

    #[test]
    fn terminates_on_shutdown_flag() {
        let proc = start_process();
        let pid = ActivationPid::from(proc.id() as i32);
        let (terminate_flag, cleanup_flag) = shutdown_flags();
        let path_hash = "abc";
        let reg_path = path_for_registry_with_entry(&path_hash);
        register_activation(&reg_path, &path_hash, pid).unwrap();
        let mut watcher = LinuxWatcher::new(
            pid,
            &reg_path,
            &path_hash,
            terminate_flag.clone(),
            cleanup_flag.clone(),
        );
        let wait_result = std::thread::scope(move |s| {
            let flag_handle = s.spawn(move || {
                std::thread::sleep(Duration::from_millis(100));
                terminate_flag.store(true, std::sync::atomic::Ordering::SeqCst);
            });
            let watcher_handle = s.spawn(move || watcher.wait_for_termination().unwrap());
            let wait_result = watcher_handle.join().unwrap();
            let _ = flag_handle.join(); // should already have terminated
            wait_result
        });
        stop_process(proc);
        assert_eq!(wait_result, WaitResult::Terminate);
    }

    #[test]
    fn terminates_on_signal_handler_flag() {
        let proc = start_process();
        let pid = ActivationPid::from(proc.id() as i32);
        let (terminate_flag, cleanup_flag) = shutdown_flags();
        let path_hash = "abc";
        let reg_path = path_for_registry_with_entry(&path_hash);
        register_activation(&reg_path, &path_hash, pid).unwrap();
        let mut watcher = LinuxWatcher::new(
            pid,
            &reg_path,
            &path_hash,
            terminate_flag.clone(),
            cleanup_flag.clone(),
        );
        let wait_result = std::thread::scope(move |s| {
            let flag_handle = s.spawn(move || {
                std::thread::sleep(Duration::from_millis(100));
                cleanup_flag.store(true, std::sync::atomic::Ordering::SeqCst);
            });
            let watcher_handle = s.spawn(move || watcher.wait_for_termination().unwrap());
            let wait_result = watcher_handle.join().unwrap();
            let _ = flag_handle.join(); // should already have terminated
            wait_result
        });
        stop_process(proc);
        assert_eq!(wait_result, WaitResult::CleanUp);
    }
}
