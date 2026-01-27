//! Types for activation state management.

use flox_core::activations::ActivationState;
use fslock::LockFile;

/// A deserialized state.json together with a lock preventing it from
/// being modified
/// TODO: there's probably a cleaner way to do this
pub type LockedActivationState = (ActivationState, LockFile);

#[cfg(test)]
pub mod test {
    use std::process::{Child, Command};
    use std::time::Duration;

    use flox_core::proc_status::{ProcStatus, pid_is_running, read_pid_status};

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
}
