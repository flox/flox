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
            match prctl::get_child_subreaper().context("failed to get child subreaper attribute")? {
                true => {
                    debug!("enabled child subreaper");
                    Ok(Self)
                },
                false => {
                    anyhow::bail!("child subreaper attribute was not set despite successful call");
                },
            }
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
        use std::thread;
        use std::time::Duration;

        use nix::errno::Errno;
        use nix::sys::wait::{WaitPidFlag, WaitStatus, waitpid};
        use nix::unistd::{ForkResult, Pid, fork};

        use super::*;

        /// Helper to create an orphaned grandchild process.
        /// Returns the grandchild's PID.
        fn create_orphaned_grandchild() -> Pid {
            match unsafe { fork() }.expect("failed to fork child") {
                ForkResult::Parent { child: child_pid } => {
                    // Wait for child to exit and get grandchild PID from exit code
                    match waitpid(child_pid, None).expect("failed to wait for child") {
                        WaitStatus::Exited(_, grandchild_raw) => {
                            let grandchild_pid = Pid::from_raw(grandchild_raw);
                            // Give grandchild time to exit and become a zombie
                            thread::sleep(Duration::from_millis(100));
                            grandchild_pid
                        },
                        status => panic!("unexpected child status: {:?}", status),
                    }
                },
                ForkResult::Child => {
                    // Child creates grandchild then exits with grandchild PID as exit code
                    match unsafe { fork() }.expect("failed to fork grandchild") {
                        ForkResult::Parent {
                            child: grandchild_pid,
                        } => {
                            std::process::exit(grandchild_pid.as_raw());
                        },
                        ForkResult::Child => std::process::exit(0),
                    }
                },
            }
        }

        /// Helper to check if a specific PID was already reaped
        fn check_pid_reaped(pid: Pid) -> bool {
            match waitpid(pid, Some(WaitPidFlag::WNOHANG)) {
                Ok(WaitStatus::StillAlive) => false,
                Err(Errno::ECHILD) => true, // Already reaped
                Ok(_) => panic!("PID {} should have been reaped already", pid),
                Err(e) => panic!("unexpected error checking PID {}: {}", pid, e),
            }
        }

        #[test]
        fn test_guard_reaps_on_drop() {
            // Fork to isolate test from other tests
            match unsafe { fork() }.expect("failed to fork") {
                ForkResult::Parent { child } => {
                    match waitpid(child, None).expect("failed to wait for child") {
                        WaitStatus::Exited(_, 0) => {},
                        status => panic!("child failed: {:?}", status),
                    }
                },
                ForkResult::Child => {
                    let grandchild_pid = {
                        let guard = SubreaperGuard::new().expect("should create subreaper guard");

                        // Verify it was actually set
                        let is_subreaper =
                            prctl::get_child_subreaper().expect("should get subreaper status");
                        assert!(is_subreaper, "should be set as subreaper");

                        let grandchild_pid = create_orphaned_grandchild();

                        // Verify the grandchild was actually reparented to us by checking
                        // that we can see it as a zombie (not yet reaped).
                        // If subreaper didn't work, waitpid would return ECHILD immediately.
                        match waitpid(grandchild_pid, Some(WaitPidFlag::WNOHANG)) {
                            Ok(WaitStatus::StillAlive) => {
                                // Good: we can wait on it, meaning it's our child now
                            },
                            Err(Errno::ECHILD) => {
                                panic!(
                                    "grandchild PID {} was not reparented to us - subreaper didn't work",
                                    grandchild_pid
                                );
                            },
                            other => panic!(
                                "unexpected status for grandchild PID {}: {:?}",
                                grandchild_pid, other
                            ),
                        }

                        // Guard drops here, should trigger reaping via Drop impl
                        drop(guard);
                        grandchild_pid
                    };

                    // After guard drops, the grandchild should have been reaped
                    assert!(
                        check_pid_reaped(grandchild_pid),
                        "guard should have reaped grandchild PID {}",
                        grandchild_pid
                    );

                    std::process::exit(0);
                },
            }
        }
    }
}
