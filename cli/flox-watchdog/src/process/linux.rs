use std::fs::read_to_string;

use tracing::{debug, warn};

/// https://man7.org/linux/man-pages/man5/proc_pid_stat.5.html
/// Process has finished but the parent hasn't reaped it yet.
const PROCFS_STATE_ZOMBIE: &str = "Z";
/// Process is dead and will transition to a zombie or disappear.
/// Technically we shouldn't see this, but just in case:
/// https://unix.stackexchange.com/a/653370
const PROCFS_STATE_DEAD: &str = "X";
const PROCFS_STATE_DEAD_COMPAT: &str = "x";

/// Process watcher for Linux based on `procfs`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ProcfsWatcher {
    pid: i32,
}

impl ProcfsWatcher {
    /// The PID will *not* be checked on instantiation so that we don't need to
    /// do additional cleanup checks before `wait_for_termination`.
    pub(crate) fn new(pid: i32) -> Self {
        Self { pid }
    }

    /// Reads the state of a process. Any failure is used as an indication that
    /// the process is no longer running.
    fn state(&self) -> Option<String> {
        let path = format!("/proc/{}/stat", self.pid);
        let stat = match read_to_string(path) {
            Ok(stat) => stat,
            Err(err) => {
                debug!(%err, self.pid, "failed to read stat, treating as not running");
                return None;
            },
        };
        // `/proc/{pid}/stat` has space separated values `pid comm state ...`
        // and we need to extract state
        let state = match stat.split_whitespace().nth(2) {
            Some(state) => state.to_string(),
            None => {
                warn!(self.pid, "failed to parse stat, treating as not running");
                return None;
            },
        };
        Some(state)
    }

    /// Returns whether the process is considered running.
    pub(crate) fn is_running(&self) -> bool {
        self.state().is_some_and(|state| {
            !matches!(
                state.as_str(),
                PROCFS_STATE_ZOMBIE | PROCFS_STATE_DEAD | PROCFS_STATE_DEAD_COMPAT
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use std::process::Command;
    use std::thread;
    use std::time::Duration;

    use super::*;

    /// Wait some attempts for the process to reach the desired state
    fn poll_until_state(watcher: &ProcfsWatcher, state: &str) {
        for _ in 0..10 {
            if let Some(s) = watcher.state() {
                if s == state {
                    break;
                }
            }
            thread::sleep(Duration::from_millis(100));
        }
    }

    #[test]
    fn test_procfswatcher_pid1_always_running() {
        let watcher = ProcfsWatcher::new(1);
        assert_eq!(watcher.is_running(), true);
    }

    #[test]
    fn test_procfswatcher_running() {
        let mut child = Command::new("sleep").arg("5").spawn().unwrap();

        let pid = child.id() as i32;
        let watcher = ProcfsWatcher::new(pid);
        assert_eq!(watcher.is_running(), true);

        child.kill().unwrap();
        child.wait().unwrap();
    }

    #[test]
    fn test_procfswatcher_not_running() {
        let mut child = Command::new("true").spawn().unwrap();

        let pid = child.id() as i32;
        let watcher = ProcfsWatcher::new(pid);
        child.wait().unwrap();

        assert_eq!(watcher.is_running(), false);
        // Verify that we hit the intended unhappy-path.
        assert_eq!(watcher.state(), None);
    }

    #[test]
    fn test_procfswatcher_zombie() {
        let mut child = Command::new("true").spawn().unwrap();

        let pid = child.id() as i32;
        let watcher = ProcfsWatcher::new(pid);
        // Allow the process to finish without calling `wait()`
        poll_until_state(&watcher, PROCFS_STATE_ZOMBIE);

        assert_eq!(watcher.is_running(), false);
        // Verify that we hit the intended unhappy-path.
        assert_eq!(watcher.state(), Some(PROCFS_STATE_ZOMBIE.to_string()));

        child.wait().unwrap();
    }
}
