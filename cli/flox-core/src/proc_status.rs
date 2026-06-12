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
    is_pid_descendant_of(std::process::id() as i32, ancestor_pid)
}

/// Check whether `pid` is `ancestor_pid` itself or a descendant of it.
///
/// Walks up the process tree from `pid` toward init, reporting a match when
/// `ancestor_pid` appears anywhere in that chain. `pid == ancestor_pid` counts
/// as a match: the self-approval guard treats the session-root process the same
/// as any process inside the session, since both are "in the session" for the
/// purpose of refusing a grant.
///
/// Used in two places that must agree: the CLI's client-side in-session refusal
/// (`pid` = the running `flox sandbox`) and the broker's server-side peer-cred
/// guard (`pid` = the control-socket peer). Sharing one predicate keeps the two
/// halves of the self-approval check from drifting apart.
pub fn is_pid_descendant_of(pid: i32, ancestor_pid: i32) -> bool {
    if pid == ancestor_pid {
        return true;
    }
    let ancestor = Pid::from_u32(ancestor_pid as u32);
    let mut system = System::new();
    let mut check_pid = Pid::from_u32(pid as u32);

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pid_is_its_own_descendant_for_the_self_approval_guard() {
        // The sandbox self-approval guard treats "is this caller in the
        // session?" as true when the caller IS the session root, not only when
        // it is strictly below it.
        let pid = std::process::id() as i32;
        assert!(is_pid_descendant_of(pid, pid));
    }

    #[test]
    fn the_current_process_is_a_descendant_of_its_parent() {
        // The current process is a descendant of its own parent — the first hop
        // of the ancestry walk. Using the live parent pid (rather than pid 1)
        // keeps the assertion robust against test-harness reparenting and the
        // platform differences in how sysinfo resolves the deeper tree.
        let pid = std::process::id() as i32;
        let mut system = System::new();
        let self_pid = Pid::from_u32(pid as u32);
        system.refresh_processes(ProcessesToUpdate::Some(&[self_pid]), false);
        let parent = system
            .process(self_pid)
            .and_then(|process| process.parent())
            .map(|parent| parent.as_u32() as i32)
            .expect("the test process must have a resolvable parent");
        assert!(is_pid_descendant_of(pid, parent));
    }

    #[test]
    fn an_unrelated_high_pid_is_not_an_ancestor() {
        // A pid that is almost certainly not running (and certainly not our
        // ancestor) yields false rather than looping or panicking.
        let pid = std::process::id() as i32;
        assert!(!is_pid_descendant_of(pid, i32::MAX - 1));
    }
}
