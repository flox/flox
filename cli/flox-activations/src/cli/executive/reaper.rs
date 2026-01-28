//! Process reaping utilities
//!
//! This module provides utilities for reaping child processes, including
//! subreaper support on Linux.

use nix::errno::Errno;
use nix::sys::wait::{WaitPidFlag, WaitStatus, waitpid};
use nix::unistd::Pid;
use tracing::debug;

/// Reap any zombie children.
///
/// This function calls `waitpid(-1, WNOHANG)` in a loop until there are no
/// more children to reap. It's safe to call even if there are no children -
/// it will simply return immediately.
///
/// On Linux with subreaper enabled, this reaps orphaned descendants that have
/// been reparented to this process. On other platforms (or without subreaper),
/// it only reaps direct children.
pub fn reap_orphaned_children() {
    loop {
        match waitpid(Pid::from_raw(-1), Some(WaitPidFlag::WNOHANG)) {
            Ok(WaitStatus::StillAlive) => break,
            Ok(status) => {
                debug!(?status, "reaped child process");
            },
            Err(Errno::ECHILD) => break,
            Err(err) => {
                debug!(%err, "unexpected error while reaping children");
                break;
            },
        }
    }
}

#[cfg(target_os = "linux")]
pub mod linux {
    //! Linux-specific subreaper functionality

    use anyhow::{Context, Result};
    use nix::sys::prctl;
    use tracing::debug;

    use super::reap_orphaned_children;

    /// RAII guard that sets the process as a subreaper on Linux and reaps
    /// orphaned children when dropped.
    ///
    /// This ensures that any orphaned descendant processes are cleaned up
    /// even if the executive exits early due to an error.
    pub struct SubreaperGuard;

    impl SubreaperGuard {
        /// Set this process as a subreaper and return a guard that will reap
        /// orphaned children when dropped.
        ///
        /// # Errors
        ///
        /// Returns an error if the subreaper attribute cannot be set or verified.
        pub fn new() -> Result<Self> {
            prctl::set_child_subreaper(true).context("failed to set child subreaper attribute")?;
            if !prctl::get_child_subreaper().context("failed to get child subreaper attribute")? {
                anyhow::bail!("child subreaper attribute was not set despite successful call");
            }

            debug!("enabled child subreaper");
            Ok(Self)
        }
    }

    impl Drop for SubreaperGuard {
        fn drop(&mut self) {
            debug!("performing final reap of orphaned children");
            reap_orphaned_children();
        }
    }

    #[cfg(test)]
    mod tests {
        use std::time::Duration;
        use std::{mem, thread};

        use nix::errno::Errno;
        use nix::sys::wait::{WaitPidFlag, WaitStatus, waitpid};
        use nix::unistd::{ForkResult, Pid, close, fork, pipe, read, write};
        use procfs::process::Process;

        use super::*;

        /// Helper to run test logic in a forked child process to isolate
        /// sub-reaper behaviour from other tests.
        /// Panics if the child exits with a non-zero code or is terminated.
        fn run_in_forked_child<F>(f: F)
        where
            F: FnOnce() + std::panic::UnwindSafe,
        {
            match unsafe { fork() }.expect("failed to fork") {
                ForkResult::Parent { child } => {
                    // Wait for child and verify it succeeded
                    let status = waitpid(child, None).expect("failed to wait for child");
                    if !matches!(status, WaitStatus::Exited(_, 0)) {
                        panic!("child process failed with status: {:?}", status);
                    }
                },
                ForkResult::Child => {
                    // Run test logic, catching panics to ensure proper exit code
                    let exit_code = match std::panic::catch_unwind(f) {
                        Ok(()) => 0,
                        Err(_) => 1,
                    };
                    std::process::exit(exit_code);
                },
            }
        }

        /// Helper to create an orphaned grandchild process.
        /// Returns the grandchild's PID.
        fn create_orphaned_grandchild() -> Pid {
            // Create a pipe to communicate grandchild PID from child to parent
            let (read_fd, write_fd) = pipe().expect("failed to create pipe");

            match unsafe { fork() }.expect("failed to fork child") {
                ForkResult::Parent { child: child_pid } => {
                    // Close write end in parent
                    close(write_fd).expect("parent failed to close write fd");

                    // Read grandchild PID from pipe
                    let mut pid_bytes = [0u8; mem::size_of::<i32>()];
                    read(&read_fd, &mut pid_bytes).expect("parent failed to read from pipe");
                    close(read_fd).expect("parent failed to close read fd");
                    let grandchild_pid = Pid::from_raw(i32::from_ne_bytes(pid_bytes));

                    // Wait for child to exit (orphaning the grandchild)
                    waitpid(child_pid, None).expect("parent failed to wait for child");

                    // Poll until grandchild becomes a zombie (exits and is reparented to us)
                    for _ in 0..50 {
                        if let Ok(process) = Process::new(grandchild_pid.as_raw())
                            && let Ok(stat) = process.stat()
                            && stat.state == 'Z'
                        {
                            break;
                        }
                        thread::sleep(Duration::from_millis(10));
                    }

                    grandchild_pid
                },
                ForkResult::Child => {
                    // Close read end in child
                    close(read_fd).expect("failed to close read fd");

                    match unsafe { fork() }.expect("failed to fork grandchild") {
                        // Child: creates grandchild then exits
                        ForkResult::Parent {
                            child: grandchild_pid,
                        } => {
                            // Send grandchild PID through pipe
                            let pid_bytes = grandchild_pid.as_raw().to_ne_bytes();
                            write(&write_fd, &pid_bytes).expect("child failed to write to pipe");
                            close(write_fd).expect("child failed to close write fd");

                            // Exit, orphaning the grandchild
                            std::process::exit(0);
                        },
                        // Grandchild: close pipe and exit
                        ForkResult::Child => {
                            close(write_fd).ok();
                            std::process::exit(0);
                        },
                    }
                },
            }
        }

        /// Assert that a specific PID was already reaped.
        fn assert_pid_reaped(pid: Pid) {
            match waitpid(pid, Some(WaitPidFlag::WNOHANG)) {
                Err(Errno::ECHILD) => {
                    // Already reaped - this is what we expect
                },
                Ok(WaitStatus::StillAlive) => {
                    // This will also reap it
                    panic!("PID {} is still alive and was not reaped", pid);
                },
                Ok(status) => {
                    panic!(
                        "PID {} should have been reaped already, but got status: {:?}",
                        pid, status
                    );
                },
                Err(e) => {
                    panic!("unexpected error checking PID {}: {}", pid, e);
                },
            }
        }

        /// Assert the grandchild was actually reparented to us.
        /// If subreaper didn't work, the parent would be init (PID 1), not us.
        fn assert_pid_direct_child(pid: Pid) {
            let current_pid = std::process::id() as i32;
            let process = Process::new(pid.as_raw())
                .expect("failed to read PID from procfs, probably already reaped");
            let stat = process.stat().expect("failed to read stat from procfs");

            assert_eq!(
                stat.ppid, current_pid,
                "PID {} was not reparented to us (parent is PID {}, expected {})",
                pid, stat.ppid, current_pid
            );
        }

        #[test]
        fn test_subreaper_guard_reaps_orphans() {
            run_in_forked_child(|| {
                let guard = SubreaperGuard::new().expect("failed to enable subreaper");
                let grandchild_pid = create_orphaned_grandchild();
                assert_pid_direct_child(grandchild_pid);
                drop(guard);
                assert_pid_reaped(grandchild_pid);
            });
        }
    }
}
