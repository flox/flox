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
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use flox_rust_sdk::models::env_registry::{activation_pids, ActivationPid};
use tracing::{debug, warn};
/// How long to wait between watcher updates.
pub const WATCHER_SLEEP_INTERVAL: Duration = Duration::from_millis(100);

type Error = anyhow::Error;

#[derive(Debug, PartialEq, Eq)]
pub enum WaitResult {
    CleanUp,
    Terminate,
}

pub trait Watcher {
    /// Block while the watcher waits for a termination or cleanup event.
    fn wait_for_termination(&mut self) -> Result<WaitResult, Error>;
    /// Instructs the watcher to update the list of PIDs that it's watching
    /// by reading the environment registry (for now).
    fn update_watchlist(&mut self) -> Result<(), Error>;
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
    pub original_pid: ActivationPid,
    pub pids_watching: HashSet<ActivationPid>,
    pub reg_path: PathBuf,
    pub hash: String,
    pub should_terminate_flag: Arc<AtomicBool>,
    pub should_clean_up_flag: Arc<AtomicBool>,
}

impl PidWatcher {
    /// Creates a new watcher that uses platform-specific mechanisms to wait
    /// for activation processes to terminate.
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

    /// Reads the state of a process on macOS using `/bin/ps`, which can report
    /// whether a process is a zombie. This is a stopgap until we someday use
    /// `libproc`. Any failure is interpreted as an indication that the process
    /// is no longer running.
    #[allow(dead_code)]
    fn read_pid_status_macos(pid: ActivationPid) -> ProcStatus {
        let pid_raw: i32 = pid.into();
        let stdout = match Command::new("/bin/ps")
            .args(["-o", "state=", "-p"])
            .arg(format!("{pid}"))
            .output()
        {
            Ok(output) => output.stdout,
            Err(err) => {
                warn!(
                    %err,
                    pid = pid_raw,
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
            debug!(
                pid = pid_raw,
                "no output from /bin/ps, treating as not running"
            );
            ProcStatus::Dead
        }
    }

    /// Tries to read the state of a process on Linux via `/proc`. Any failure
    /// is interpreted as an indication that the process is no longer running.
    #[allow(dead_code)]
    fn read_pid_status_linux(pid: ActivationPid) -> ProcStatus {
        let path = format!("/proc/{pid}/stat");
        let pid_raw: i32 = pid.into();
        let stat = match read_to_string(path) {
            Ok(stat) => stat,
            Err(err) => {
                warn!(
                    %err,
                    pid = pid_raw,
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
                pid = pid_raw,
                "failed to parse /proc/<pid>/stat, treating as not running"
            );
            ProcStatus::Dead
        }
    }

    /// Returns the status of the provided PID.
    fn read_pid_status(pid: ActivationPid) -> ProcStatus {
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        panic!("unsupported operating system");

        #[cfg(target_os = "linux")]
        let status = Self::read_pid_status_linux(pid);

        #[cfg(target_os = "macos")]
        let status = Self::read_pid_status_macos(pid);

        status
    }

    /// Returns whether the process is considered running.
    pub fn pid_is_running(pid: ActivationPid) -> bool {
        Self::read_pid_status(pid) == ProcStatus::Running
    }

    fn prune_terminations(&mut self) {
        self.pids_watching.retain(|&pid| Self::pid_is_running(pid));
    }
}

impl Watcher for PidWatcher {
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
        // Add all PIDs, even if they're dead, but then immediately remove them
        self.pids_watching.extend(all_registered_pids);
        self.prune_terminations();
        Ok(())
    }

    fn should_clean_up(&self) -> Result<bool, super::Error> {
        Ok(self.pids_watching.is_empty())
    }
}

#[cfg(test)]
mod test {
    use std::collections::HashSet;
    use std::path::PathBuf;
    use std::process::{Child, Command};
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    use flox_rust_sdk::models::env_registry::{register_activation, EnvRegistry, RegistryEntry};
    use tempfile::NamedTempFile;

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

    /// Writes a registry to a temporary file, adding an entry for the provided
    /// path hash
    pub fn path_for_registry_with_entry(path_hash: impl AsRef<str>) -> NamedTempFile {
        let path = NamedTempFile::new().unwrap();
        let mut reg = EnvRegistry::default();
        let entry = RegistryEntry {
            path_hash: String::from(path_hash.as_ref()),
            path: PathBuf::from("foo"),
            envs: vec![],
            activations: HashSet::new(),
        };
        reg.entries.push(entry);
        let string = serde_json::to_string(&reg).unwrap();
        std::fs::write(&path, string).unwrap();
        path
    }

    /// Wait some attempts for the process to reach the desired state
    fn poll_until_state(state: ProcStatus, pid: ActivationPid) {
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
        assert!(PidWatcher::pid_is_running(1.into()));
    }

    #[test]
    fn detects_running_or_not_running_process() {
        let proc = start_process();
        let pid = proc.id() as i32;
        assert!(PidWatcher::pid_is_running(pid.into()));
        stop_process(proc);
        assert!(!PidWatcher::pid_is_running(pid.into()));
    }

    #[test]
    fn detects_zombie() {
        let mut proc = Command::new("true").spawn().unwrap();
        let pid = proc.id() as i32;
        poll_until_state(ProcStatus::Zombie, pid.into());
        assert!(!PidWatcher::pid_is_running(pid.into()));
        assert_eq!(PidWatcher::read_pid_status(pid.into()), ProcStatus::Zombie);
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
            PidWatcher::new(pid1, &reg_path, &path_hash, terminate_flag, cleanup_flag);
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
        let mut watcher = PidWatcher::new(
            pid,
            &reg_path,
            &path_hash,
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
        let mut watcher = PidWatcher::new(
            pid,
            &reg_path,
            &path_hash,
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
        assert_eq!(wait_result, WaitResult::CleanUp);
    }
}
