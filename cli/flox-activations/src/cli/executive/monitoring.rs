//! Executive monitoring loop for activation lifecycle management.
//!
//! This module monitors activation processes and performs cleanup when all
//! processes have terminated.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::{Context, Result, bail};
use flox_core::activations::{activation_state_dir_path, read_activations_json, state_json_path};
use flox_core::traceable_path;
use nix::libc::{SIGCHLD, SIGINT, SIGQUIT, SIGTERM, SIGUSR1};
use nix::unistd::{getpgid, getpid, setsid};
use signal_hook::iterator::Signals;
use tracing::{debug, error, info, instrument};

use super::watcher::{LockedActivationState, PidWatcher, WaitResult, Watcher};
use crate::process_compose::process_compose_down;

type Error = anyhow::Error;

#[derive(Debug, Clone)]
pub struct Args {
    /// The path to the .flox directory
    pub dot_flox_path: PathBuf,

    /// The path to the Flox environment symlink
    pub flox_env: PathBuf,

    /// The path to the runtime directory keeping activation data
    pub runtime_dir: PathBuf,

    /// The path to the process-compose socket
    pub socket_path: PathBuf,
}

#[instrument("monitoring", err(Debug), skip_all)]
pub fn run(args: Args) -> Result<(), Error> {
    let span = tracing::Span::current();
    span.record("flox_env", traceable_path(&args.flox_env));
    span.record("runtime_dir", traceable_path(&args.runtime_dir));
    span.record("socket", traceable_path(&args.socket_path));
    debug!("starting");

    ensure_process_group_leader()
        .context("failed to ensure executive is detached from terminal")?;

    // Set the signal handlers
    let should_clean_up = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(SIGUSR1, Arc::clone(&should_clean_up))
        .context("failed to set SIGUSR1 signal handler")?;
    let should_terminate = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(SIGINT, Arc::clone(&should_terminate))
        .context("failed to set SIGINT signal handler")?;
    signal_hook::flag::register(SIGTERM, Arc::clone(&should_terminate))
        .context("failed to set SIGTERM signal handler")?;
    signal_hook::flag::register(SIGQUIT, Arc::clone(&should_terminate))
        .context("failed to set SIGQUIT signal handler")?;
    // This compliments the SubreaperGuard setup by `flox_activations::executive`
    // WARNING: You cannot reliably use Command::wait after we've entered the
    // monitoring loop, including concurrent threads like GCing logs, because
    // children will be reaped automatically.
    let should_reap = Signals::new([SIGCHLD])?;

    run_inner(args, should_terminate, should_clean_up, should_reap)
}

/// Function to be used for unit tests that doesn't do weird process stuff
pub(super) fn run_inner(
    args: Args,
    should_terminate: Arc<AtomicBool>,
    should_clean_up: Arc<AtomicBool>,
    should_reap: Signals,
) -> Result<(), Error> {
    let state_json_path = state_json_path(&args.runtime_dir, &args.dot_flox_path);

    let mut watcher = PidWatcher::new(
        state_json_path.clone(),
        args.dot_flox_path.clone(),
        args.runtime_dir.clone(),
        should_terminate,
        should_clean_up,
        should_reap,
    );

    debug!(
        socket = traceable_path(&args.socket_path),
        exists = &args.socket_path.exists(),
        "checked socket"
    );

    info!(
        this_pid = nix::unistd::getpid().as_raw(),
        "executive is on duty"
    );

    match watcher.wait_for_termination() {
        Ok(WaitResult::CleanUp(locked_activations)) => {
            // Exit
            info!("running cleanup after all PIDs terminated");
            cleanup(
                locked_activations,
                &args.socket_path,
                activation_state_dir_path(&args.runtime_dir, &args.dot_flox_path),
            )
            .context("cleanup failed")?;
        },
        Ok(WaitResult::Terminate) => {
            // If we get a SIGINT/SIGTERM/SIGQUIT/SIGKILL we leave behind the activation in the registry,
            // but there's not much we can do about that because we don't know who sent us one of those
            // signals or why.
            bail!("received stop signal, exiting without cleanup");
        },
        Err(err) => {
            info!("running cleanup after error");
            let (activations_json, lock) = read_activations_json(&state_json_path)?;
            let Some(activations) = activations_json else {
                bail!("executive shouldn't be running when state.json doesn't exist");
            };
            let _ = cleanup(
                (activations, lock),
                &args.socket_path,
                activation_state_dir_path(&args.runtime_dir, &args.dot_flox_path),
            );
            bail!(err.context("failed while waiting for termination"))
        },
    }

    Ok(())
}

/// Shutdown `process-compose` if running and remove all activation state.
/// To be called when there are no longer any PIDs attached.
fn cleanup(
    locked_activations: LockedActivationState,
    socket_path: impl AsRef<Path>,
    activation_state_dir_path: impl AsRef<Path>,
) -> Result<()> {
    info!("running cleanup");

    let (activations_json, _hold_the_lock) = locked_activations;

    if !activations_json.attached_pids_is_empty() {
        unreachable!("cleanup should only be called when there are no more attached PIDs");
    }
    let socket_path = socket_path.as_ref();
    if socket_path.exists() {
        if let Err(err) = process_compose_down(socket_path) {
            error!(%err, "failed to run process-compose shutdown command");
        }
        info!("shut down process-compose");
    } else {
        info!(reason = "no socket", "did not shut down process-compose");
    }

    // Atomically remove the activation state directory
    // We want to avoid a race where remove_dir_all removes the lock before
    // removing activation state dir,
    // and then another activation creates a lock and causes remove_dir_all to
    // fail.
    let activation_state_dir_path = activation_state_dir_path.as_ref();
    let cleanup_path =
        activation_state_dir_path.with_extension(format!("cleanup.{}", std::process::id()));
    fs::rename(activation_state_dir_path, &cleanup_path)
        .context("couldn't rename activations dir for cleanup")?;
    fs::remove_dir_all(&cleanup_path).context("couldn't remove activations dir")?;

    info!("finished cleanup");

    Ok(())
}

/// We want to make sure that the executive is detached from the terminal in case it sends
/// any signals to the activation. A terminal sends signals to all processes in a process group,
/// and we want to make sure that the executive is in its own process group to avoid receiving any
/// signals intended for the shell.
///
/// From local testing I haven't been able to deliver signals to the executive by sending signals to
/// the activation, so this is more of a "just in case" measure.
fn ensure_process_group_leader() -> Result<(), Error> {
    let pid = getpid();
    // Trivia:
    // You can't create a new session if you're already a session leader, the reason being that
    // the other processes in the group aren't automatically moved to the new session. You're supposed
    // to have this invariant: all processes in a process group share the same controlling terminal.
    // If you were able to create a new session as session leader and leave behind the other processes
    // in the group in the old session, it would be possible for processes in this group to be in two
    // different sessions and therefore have two different controlling terminals.
    if pid != getpgid(None).context("failed to get process group leader")? {
        setsid().context("failed to create new session")?;
    }
    Ok(())
}

#[cfg(test)]
mod test {
    use flox_core::activate::mode::ActivateMode;
    use flox_core::activations::test_helpers::write_activation_state;
    use flox_core::activations::{ActivationState, StartOrAttachResult};

    use super::super::watcher::test::{shutdown_flags, start_process, stop_process};
    use super::*;

    #[test]
    fn cleanup_removes_state_directory() {
        let temp_dir = tempfile::tempdir().unwrap();
        let runtime_dir = temp_dir.path();
        let dot_flox_path = PathBuf::from(".flox");
        let flox_env = dot_flox_path.join("run/test");
        let store_path = "store_path".to_string();

        let proc = start_process();
        let pid = proc.id() as i32;

        // Create an ActivationState with one PID attached
        let mut state = ActivationState::new(&ActivateMode::default(), &dot_flox_path, &flox_env);
        let result = state.start_or_attach(pid, &store_path);
        let StartOrAttachResult::Start { start_id, .. } = result else {
            panic!("Expected Start")
        };
        state.set_ready(&start_id);

        // Write state to disk
        write_activation_state(runtime_dir, &dot_flox_path, state);

        let activation_state_directory = activation_state_dir_path(runtime_dir, &dot_flox_path);
        assert!(
            activation_state_directory.exists(),
            "state directory should exist before cleanup"
        );

        stop_process(proc);

        let args = Args {
            dot_flox_path: dot_flox_path.clone(),
            flox_env: dot_flox_path.clone(),
            runtime_dir: runtime_dir.to_path_buf(),
            socket_path: PathBuf::from("/does_not_exist"),
        };

        let (terminate_flag, cleanup_flag, reap_flag) = shutdown_flags();
        run_inner(args, terminate_flag, cleanup_flag, reap_flag).unwrap();

        // Verify state directory is completely removed after cleanup
        assert!(
            !activation_state_directory.exists(),
            "state directory should be removed after cleanup"
        );
    }
}
