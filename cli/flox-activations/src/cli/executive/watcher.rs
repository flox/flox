//! This module watches PIDs and uses platform specific mechanisms to determine
//! when processes are runnable, zombies, or terminated.
//!
//! On Linux we read `/proc`. See the
//! [man page](https://man7.org/linux/man-pages/man5/proc_pid_stat.5.html) for
//! more details.
//!
//! On macOS we slum it and call `/bin/ps` rather than using the private `libproc.h`
//! API, but mostly for build-complexity reasons.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use flox_core::activations::{ActivationState, read_activations_json, write_activations_json};
use flox_core::proc_status::pid_is_running;
use fslock::LockFile;
use time::OffsetDateTime;
use tracing::trace;

type Error = anyhow::Error;

/// A deserialized state.json together with a lock preventing it from
/// being modified
/// TODO: there's probably a cleaner way to do this
pub type LockedActivationState = (ActivationState, LockFile);

/// Watches PIDs attached to an activation and updates state.json when they terminate.
#[derive(Debug)]
pub struct PidWatcher {
    state_json_path: PathBuf,
    activation_state_dir: PathBuf,
}

impl PidWatcher {
    /// Creates a new watcher for the given activation.
    pub fn new(state_json_path: PathBuf, activation_state_dir: PathBuf) -> Self {
        Self {
            state_json_path,
            activation_state_dir,
        }
    }

    /// Reload and check the list of PIDs for an activation.
    ///
    /// Returns `Some(LockedActivationState)` if all PIDs have terminated and
    /// cleanup should proceed.
    /// Returns `None` if there are still active PIDs.
    pub fn cleanup_pids(&mut self) -> Result<Option<LockedActivationState>, Error> {
        let (activations_json, lock) = read_activations_json(&self.state_json_path)?;
        let Some(mut activations) = activations_json else {
            bail!("executive shouldn't be running when state.json doesn't exist");
        };

        let now = OffsetDateTime::now_utc();
        let (empty_start_ids, modified) = activations.cleanup_pids(pid_is_running, now);

        // If there are no more attached PIDs for any start, return early and
        // cleanup the entirety of the activation state directory
        if activations.attached_pids_is_empty() {
            return Ok(Some((activations, lock)));
        }

        // Cleanup empty start IDs
        //
        // We might want to skip this if start_id is the same as that in ready,
        // since otherwise we'll do another start of the same environment when:
        // 1. There were still some activations of the environment
        // and
        // 2. The environment was not modified
        // But I think for now it's simpler to just treat all start_ids the same.
        for start_id in empty_start_ids {
            let state_dir = start_id.start_state_dir(&self.activation_state_dir)?;
            trace!(?state_dir, "removing empty activation state dir");
            std::fs::remove_dir_all(state_dir).context("failed to remove start state dir")?;
        }

        if modified {
            trace!(?activations, "writing PID changes to activation");
            self.update_activations_file(activations, lock)?;
        }

        Ok(None)
    }

    /// Update the `state.json` file with the current list of running PIDs.
    fn update_activations_file(
        &self,
        activations: ActivationState,
        lock: LockFile,
    ) -> Result<(), Error> {
        write_activations_json(&activations, &self.state_json_path, lock)
    }
}

#[cfg(test)]
pub mod test {
    use std::collections::BTreeMap;
    use std::process::{Child, Command};
    use std::time::Duration;

    use flox_core::activate::mode::ActivateMode;
    use flox_core::activations::test_helpers::{read_activation_state, write_activation_state};
    use flox_core::activations::{StartOrAttachResult, activation_state_dir_path, state_json_path};
    use flox_core::proc_status::{ProcStatus, pid_is_running, read_pid_status};

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

    #[test]
    fn cleanup_returns_when_all_pids_terminate() {
        let runtime_dir = tempfile::tempdir().unwrap();
        let dot_flox_path = PathBuf::from(".flox");
        let flox_env = dot_flox_path.join("run/test");
        let store_path = "store_path".to_string();

        let proc1 = start_process();
        let pid1 = proc1.id() as i32;
        let proc2 = start_process();
        let pid2 = proc2.id() as i32;

        // Create an ActivationState with two PIDs attached to the same start_id
        let mut state =
            ActivationState::new(&ActivateMode::default(), Some(&dot_flox_path), &flox_env);
        let result = state.start_or_attach(pid1, &store_path);
        let StartOrAttachResult::Start { start_id, .. } = result else {
            panic!("Expected Start")
        };
        state.set_ready(&start_id);
        let result = state.start_or_attach(pid2, &store_path);
        assert!(matches!(result, StartOrAttachResult::Attach { .. }));

        write_activation_state(runtime_dir.path(), &dot_flox_path, state);

        let activation_state_dir = activation_state_dir_path(runtime_dir.path(), &dot_flox_path);
        let state_json_path = state_json_path(&activation_state_dir);
        let mut watcher = PidWatcher::new(state_json_path, activation_state_dir);

        // Terminate both processes
        stop_process(proc1);
        stop_process(proc2);

        let (state, _lock) = watcher
            .cleanup_pids()
            .unwrap()
            .expect("should return cleanup result");
        assert_eq!(
            state.attachments_by_start_id(),
            BTreeMap::new(),
            "should return empty state for cleanup"
        );
    }

    /// When an attachment to a start exits, its PID is removed if its the first
    /// PID in the list
    #[test]
    fn terminated_pid_removed_if_first() {
        let runtime_dir = tempfile::tempdir().unwrap();
        let dot_flox_path = PathBuf::from(".flox");
        let flox_env = dot_flox_path.join("run/test");
        let store_path = "store_path".to_string();

        // Start and set ready pid1
        let proc1 = start_process();
        let pid1 = proc1.id() as i32;
        let mut state =
            ActivationState::new(&ActivateMode::default(), Some(&dot_flox_path), &flox_env);
        let result = state.start_or_attach(pid1, &store_path);
        let StartOrAttachResult::Start { start_id, .. } = result else {
            panic!("Expected Start")
        };

        state.set_ready(&start_id);
        let proc2 = start_process();
        let pid2 = proc2.id() as i32;
        state.start_or_attach(pid2, &store_path);

        // Verify both PIDs are initially present
        let initial_pids = state.attached_pids_running();
        assert_eq!(
            initial_pids,
            vec![pid1, pid2],
            "both pids should be running and present"
        );

        write_activation_state(runtime_dir.path(), &dot_flox_path, state);

        // The cleanup logic is order sensitive,
        // so proc2 wouldn't be cleaned up if we stopped it instead of proc1
        stop_process(proc1);

        let activation_state_dir = activation_state_dir_path(runtime_dir.path(), &dot_flox_path);
        let state_json_path = state_json_path(&activation_state_dir);
        let mut watcher = PidWatcher::new(state_json_path, activation_state_dir);

        // Call cleanup_pids to process the terminated PID
        let result = watcher.cleanup_pids().unwrap();
        assert!(result.is_none(), "should not cleanup while pid2 is running");

        // Check state while proc2 is still running
        let intermediate_state = read_activation_state(runtime_dir.path(), &dot_flox_path);
        let intermediate_attachments = intermediate_state.attachments_by_start_id();
        assert_eq!(
            BTreeMap::from([(start_id.clone(), vec![(pid2, None)])]),
            intermediate_attachments,
            "only pid2 should be running and present after proc1 terminated"
        );

        // Clean up
        stop_process(proc2);
        let (state, _lock) = watcher
            .cleanup_pids()
            .unwrap()
            .expect("should return cleanup result");
        assert_eq!(
            state.attachments_by_start_id(),
            BTreeMap::new(),
            "should return empty state for cleanup"
        );
    }

    /// After all attachments to a start exit, start state directory is removed
    #[test]
    fn cleans_up_start_state_directory() {
        let runtime_dir = tempfile::tempdir().unwrap();
        let dot_flox_path = PathBuf::from(".flox");
        let flox_env = dot_flox_path.join("run/test");
        let store_path_1 = "store_path_1".to_string();
        let store_path_2 = "store_path_2".to_string();

        let proc1 = start_process();
        let pid1 = proc1.id() as i32;
        let proc2 = start_process();
        let pid2 = proc2.id() as i32;

        // Start and set ready for store_path_1
        let mut state =
            ActivationState::new(&ActivateMode::default(), Some(&dot_flox_path), &flox_env);
        let result = state.start_or_attach(pid1, &store_path_1);
        let StartOrAttachResult::Start {
            start_id: start_id_1,
            ..
        } = result
        else {
            panic!("Expected Start")
        };
        state.set_ready(&start_id_1);

        // Start and set ready for store_path_2
        let result = state.start_or_attach(pid2, &store_path_2);
        let StartOrAttachResult::Start {
            start_id: start_id_2,
            ..
        } = result
        else {
            panic!("Expected Start")
        };
        state.set_ready(&start_id_2);

        write_activation_state(runtime_dir.path(), &dot_flox_path, state);

        // Create both state directories
        let activation_state_dir = activation_state_dir_path(runtime_dir.path(), &dot_flox_path);
        let state_dir_1 = start_id_1.start_state_dir(&activation_state_dir).unwrap();
        let state_dir_2 = start_id_2.start_state_dir(&activation_state_dir).unwrap();
        std::fs::create_dir_all(&state_dir_1).unwrap();
        std::fs::create_dir_all(&state_dir_2).unwrap();
        assert!(state_dir_1.exists());
        assert!(state_dir_2.exists());

        let state_json_path = state_json_path(&activation_state_dir);
        let mut watcher = PidWatcher::new(state_json_path, activation_state_dir);

        // Terminate proc1 and call cleanup_pids
        stop_process(proc1);
        let result = watcher.cleanup_pids().unwrap();
        assert!(result.is_none(), "should not cleanup while pid2 is running");

        // Verify state_dir_1 has been removed but state_dir_2 still exists
        assert!(!state_dir_1.exists(), "state directory 1 should be removed");
        assert!(state_dir_2.exists(), "state directory 2 should still exist");

        // Clean up
        stop_process(proc2);
        let (state, _lock) = watcher
            .cleanup_pids()
            .unwrap()
            .expect("should return cleanup result");
        assert_eq!(
            state.attachments_by_start_id(),
            BTreeMap::new(),
            "should return empty state for cleanup"
        );
    }
}
