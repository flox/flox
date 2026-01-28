use std::fs::read_to_string;
use std::num::ParseIntError;
use std::process::Command;

use sysinfo::{Pid, ProcessesToUpdate, System};
use tracing::{debug, trace, warn};

#[derive(Debug, thiserror::Error)]
pub enum ProcStatusError {
    #[error("failed to list processes")]
    RunCommand(std::io::Error),
    #[error("failed to list processes")]
    PsFailed,
    #[error("failed to list processes")]
    ParsePid(ParseIntError),
    #[error("failed to list processes")]
    ParsePsOutput,
}

/// The state that a process is in.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ProcStatus {
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
pub fn read_pid_status(pid: i32) -> ProcStatus {
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    panic!("unsupported operating system");

    #[cfg(target_os = "linux")]
    let status = read_pid_status_linux(pid);

    #[cfg(target_os = "macos")]
    let status = read_pid_status_macos(pid);

    status
}

/// Returns whether the process is considered running.
pub fn pid_is_running(pid: i32) -> bool {
    read_pid_status(pid) == ProcStatus::Running
}

/// Check if the current process is a descendant of the given PID.
///
/// Walks up the process tree from the current process to see if `ancestor_pid`
/// is in the parent chain.
pub fn is_descendant_of(ancestor_pid: i32) -> bool {
    let ancestor = Pid::from_u32(ancestor_pid as u32);
    let mut system = System::new();
    let mut check_pid = Pid::from_u32(std::process::id());

    // Safety limit - process trees shouldn't be deeper than this.
    for _ in 0..256 {
        // Don't refresh all to avoid unnecessary overhead.
        system.refresh_processes(ProcessesToUpdate::Some(&[check_pid]), false);
        let Some(process) = system.process(check_pid) else {
            return false;
        };
        let Some(parent_pid) = process.parent() else {
            return false;
        };

        if parent_pid == ancestor {
            return true;
        }
        if parent_pid.as_u32() <= 1 {
            return false; // Reached init/kernel
        }
        check_pid = parent_pid;
    }
    false
}
