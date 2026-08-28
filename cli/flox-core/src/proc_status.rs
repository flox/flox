use std::fs::read_to_string;

use sysinfo::{Pid, ProcessesToUpdate, System};
use tracing::{trace, warn};

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

/// `PROC_PIDT_SHORTBSDINFO` and `proc_bsdshortinfo` copied verbatim from
/// libc 0.2.189 (`src/unix/bsd/apple/mod.rs`): the workspace's libc is
/// exact-pinned to 0.2.180 by nix 0.31, which does not export them yet.
/// Drop these once the workspace libc reaches 0.2.189.
#[cfg(target_os = "macos")]
const PROC_PIDT_SHORTBSDINFO: nix::libc::c_int = 13;

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Copy, Clone)]
#[allow(non_camel_case_types)]
struct proc_bsdshortinfo {
    pub pbsi_pid: u32,
    pub pbsi_ppid: u32,
    pub pbsi_pgid: u32,
    pub pbsi_status: u32,
    pub pbsi_comm: [nix::libc::c_char; nix::libc::MAXCOMLEN],
    pub pbsi_flags: u32,
    pub pbsi_uid: nix::libc::uid_t,
    pub pbsi_gid: nix::libc::gid_t,
    pub pbsi_ruid: nix::libc::uid_t,
    pub pbsi_rgid: nix::libc::gid_t,
    pub pbsi_svuid: nix::libc::uid_t,
    pub pbsi_svgid: nix::libc::gid_t,
    pbsi_rfu: u32,
}

/// Reads the state of a process on macOS via `proc_pidinfo`, with a
/// `kill(pid, 0)` probe to tell zombies apart from reaped processes.
/// Direct syscalls rather than spawning `/bin/ps`: hardened boundaries a
/// session-wrap plugin re-enters the activation under (e.g. Seatbelt
/// profiles) deny exec'ing `ps` while permitting process-info syscalls on
/// processes inside the boundary. Any failure is interpreted as an
/// indication that the process is no longer running.
#[cfg(target_os = "macos")]
fn read_pid_status_macos(pid: i32) -> ProcStatus {
    use nix::libc;

    let mut info = std::mem::MaybeUninit::<proc_bsdshortinfo>::uninit();
    let size = std::mem::size_of::<proc_bsdshortinfo>() as libc::c_int;
    let ret = unsafe {
        libc::proc_pidinfo(
            pid,
            PROC_PIDT_SHORTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    if ret == size {
        let info = unsafe { info.assume_init() };
        // Current Darwin reports zombies as ESRCH (the probe below), but
        // older releases returned them here with SZOMB.
        return if info.pbsi_status == libc::SZOMB {
            ProcStatus::Zombie
        } else {
            ProcStatus::Running
        };
    }
    // A zombie still exists until reaped, so signaling it succeeds; a
    // reaped process is ESRCH. EPERM means the process exists but is not
    // ours to signal or query, which without a readable state also lands
    // on "existing but not runnable".
    match nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None) {
        Err(nix::errno::Errno::ESRCH) => ProcStatus::Dead,
        Ok(()) => ProcStatus::Zombie,
        Err(err) => {
            warn!(%err, pid, "could not probe process, treating as zombie");
            ProcStatus::Zombie
        },
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
