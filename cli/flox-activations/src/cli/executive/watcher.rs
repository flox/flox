//! This module watches PIDs and uses platform specific mechanisms to determine
//! when processes are runnable, zombies, or terminated.
//!
//! On Linux we read `/proc`. See the
//! [man page](https://man7.org/linux/man-pages/man5/proc_pid_stat.5.html) for
//! more details.
//!
//! On macOS we slum it and call `/bin/ps` rather than using the private `libproc.h`
//! API, but mostly for build-complexity reasons.

use std::path::Path;

use anyhow::Result;
use flox_core::activations::{ActivationState, write_activations_json};
use flox_core::proc_status::pid_is_running;
use fslock::LockFile;
use time::OffsetDateTime;
use tracing::trace;

type Error = anyhow::Error;

/// A deserialized state.json together with a lock preventing it from
/// being modified
/// TODO: there's probably a cleaner way to do this
pub type LockedActivationState = (ActivationState, LockFile);

/// Check if the provided PID is still running and clean it up if not.
///
/// Takes the state already locked rather than reading it, so that whether
/// state.json still exists is decided by the caller. Every arm of the event
/// loop answers that the same way, and it is not this function's policy to set.
///
/// Returns `Some(LockedActivationState)` if all PIDs have terminated and
/// cleanup should proceed.
/// Returns `None` if there are still active PIDs.
pub fn cleanup_pid(
    locked_activations: LockedActivationState,
    state_json_path: &Path,
    pid: i32,
) -> Result<Option<LockedActivationState>, Error> {
    let (mut activations, lock) = locked_activations;

    let now = OffsetDateTime::now_utc();
    let (empty_start_id, modified) = activations.cleanup_pid(pid, pid_is_running, now);

    // If there are no more attached PIDs for any start, return early and
    // cleanup the entirety of the activation state directory
    if activations.attached_pids_is_empty() {
        return Ok(Some((activations, lock)));
    }

    // A start emptied by this PID is not torn down here: the caller sweeps it
    // (running hook.on-deactivate first) after this function's lock has been
    // released.
    trace!(
        ?empty_start_id,
        "PID cleanup left start with no attachments"
    );

    if modified {
        trace!(?activations, "writing PID changes to activation");
        write_activations_json(&activations, state_json_path, lock)?;
    }

    Ok(None)
}

#[cfg(test)]
pub mod test {
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::process::{Child, Command};
    use std::time::Duration;

    use flox_core::activate::context::{AttachCtx, AttachProjectCtx};
    use flox_core::activate::mode::ActivateMode;
    use flox_core::activations::test_helpers::write_activation_state;
    use flox_core::activations::{
        StartOrAttachResult,
        activation_state_dir_path,
        read_activations_json,
        state_json_path,
    };
    use flox_core::proc_status::{ProcStatus, pid_is_running, read_pid_status};

    use super::*;
    use crate::on_deactivate::{orphaned_start_ids, sweep_orphaned_starts};

    /// Create minimal context for testing.
    /// The actual values don't matter since tests don't trigger SIGUSR1.
    pub fn test_context(dot_flox_path: &Path, flox_env: &str) -> (AttachCtx, AttachProjectCtx) {
        let attach = AttachCtx {
            env: flox_env.to_string(),
            env_description: "test".to_string(),
            env_cache: dot_flox_path.join("cache"),
            interpreter_path: PathBuf::from("/nix/store/fake"),
            prompt_color_1: "".to_string(),
            prompt_color_2: "".to_string(),
            flox_prompt_environments: "".to_string(),
            set_prompt: false,
            flox_env_cuda_detection: "".to_string(),
            add_sbin: false,
            flox_active_environments: "".to_string(),
        };
        let project = AttachProjectCtx {
            env_project: dot_flox_path.to_path_buf(),
            dot_flox_path: dot_flox_path.to_path_buf(),
            flox_env_log_dir: PathBuf::from("/tmp/test_log_dir"),
            flox_services_socket: PathBuf::from("/does_not_exist"),
            process_compose_bin: PathBuf::from("/nix/store/fake-process-compose"),
            services_to_start: Vec::new(),
        };
        (attach, project)
    }

    /// Read and lock the state the way the event loop does before calling
    /// [`cleanup_pid`].
    fn locked_state(state_json_path: &Path) -> LockedActivationState {
        let (activations, lock) = read_activations_json(state_json_path).unwrap();
        (activations.expect("state.json should exist"), lock)
    }

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

        // Clean up first PID - should not trigger full cleanup yet
        stop_process(proc1);
        let result = cleanup_pid(locked_state(&state_json_path), &state_json_path, pid1).unwrap();
        assert!(result.is_none(), "should not cleanup after first PID");

        // Clean up second PID - should trigger full cleanup
        stop_process(proc2);
        let (state, _lock) = cleanup_pid(locked_state(&state_json_path), &state_json_path, pid2)
            .unwrap()
            .expect("should return cleanup result");
        assert_eq!(
            state.attachments_by_start_id(),
            BTreeMap::new(),
            "should return empty state after cleanup"
        );
    }

    /// After all attachments to a start exit, `cleanup_pid` hands the start
    /// state directory off to the caller's sweep, which removes it.
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

        // Create both state directories with their start id markers
        let activation_state_dir = activation_state_dir_path(runtime_dir.path(), &dot_flox_path);
        let state_dir_1 = start_id_1.start_state_dir(&activation_state_dir).unwrap();
        let state_dir_2 = start_id_2.start_state_dir(&activation_state_dir).unwrap();
        std::fs::create_dir_all(&state_dir_1).unwrap();
        std::fs::create_dir_all(&state_dir_2).unwrap();
        start_id_1
            .write_to_start_state_dir(&activation_state_dir)
            .unwrap();
        start_id_2
            .write_to_start_state_dir(&activation_state_dir)
            .unwrap();
        assert!(state_dir_1.exists());
        assert!(state_dir_2.exists());

        let state_json_path = state_json_path(&activation_state_dir);

        // Terminate proc1 and call cleanup_pid
        stop_process(proc1);
        let result = cleanup_pid(locked_state(&state_json_path), &state_json_path, pid1).unwrap();
        assert!(result.is_none(), "should not cleanup while pid2 is running");

        // cleanup_pid leaves the emptied start's directory for the caller's
        // sweep, which runs hook.on-deactivate before removing it.
        assert!(
            state_dir_1.exists(),
            "state directory 1 should be handed off to the sweep, not removed"
        );

        let (activations, lock) = read_activations_json(&state_json_path).unwrap();
        let orphaned = orphaned_start_ids(&activation_state_dir, &activations.unwrap());
        drop(lock);
        let (attach, project) = test_context(&dot_flox_path, &flox_env.to_string_lossy());
        sweep_orphaned_starts(0, &attach, &project, &activation_state_dir, orphaned);

        // Verify state_dir_1 has been removed but state_dir_2 still exists
        assert!(!state_dir_1.exists(), "state directory 1 should be removed");
        assert!(state_dir_2.exists(), "state directory 2 should still exist");

        // Clean up
        stop_process(proc2);
        let (state, _lock) = cleanup_pid(locked_state(&state_json_path), &state_json_path, pid2)
            .unwrap()
            .expect("should return cleanup result");
        assert_eq!(
            state.attachments_by_start_id(),
            BTreeMap::new(),
            "should return empty state for cleanup"
        );
    }
}
