use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Args;
use flox_core::activate::context::{ActivateCtx, AttachCtx};
use flox_core::activations::{
    activation_state_dir_path,
    read_activations_json,
    state_json_path,
    write_activations_json,
};
use flox_core::traceable_path;
use log_gc::{spawn_heartbeat_log, spawn_logs_gc_threads};
use nix::sys::signal::Signal::SIGUSR1;
use nix::sys::signal::kill;
use nix::unistd::{Pid, getpgid, getpid, setsid};
use pid_monitor::{PidEvent, PidMonitorCoordinator};
use reaper::reap_orphaned_children;
use serde::{Deserialize, Serialize};
use tracing::{debug, debug_span, error, info, instrument};
use watcher::LockedActivationState;

use crate::cli::activate::NO_REMOVE_ACTIVATION_FILES;
use crate::logger;
use crate::process_compose::{process_compose_down, start_process_compose_no_services};

mod log_gc;
mod pid_monitor;
mod reaper;
mod watcher;
// TODO: Re-enable sentry after fixing OpenSSL dependency issues
// mod sentry;

#[cfg(target_os = "linux")]
use reaper::linux::SubreaperGuard;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutiveCtx {
    pub context: ActivateCtx,
    pub parent_pid: i32,
}

#[derive(Debug, Args)]
pub struct ExecutiveArgs {
    /// .flox directory path
    // This isn't consumed and serves only to identify in process listings which
    // environment the executive is responsible for.
    #[arg(long)]
    pub dot_flox_path: PathBuf,

    /// Path to JSON file containing executive context
    #[arg(long)]
    pub executive_ctx: PathBuf,
}

impl ExecutiveArgs {
    pub fn handle(self, subsystem_verbosity: Option<u32>) -> Result<(), anyhow::Error> {
        let contents = fs::read_to_string(&self.executive_ctx)?;
        let ExecutiveCtx {
            context,
            parent_pid,
        } = serde_json::from_str(&contents)?;
        if !std::env::var(NO_REMOVE_ACTIVATION_FILES).is_ok_and(|val| val == "true") {
            fs::remove_file(&self.executive_ctx)?;
        }

        // Set as subreaper immediately. The guard ensures cleanup on all exit paths.
        #[cfg(target_os = "linux")]
        let _subreaper_guard = SubreaperGuard::new()?;

        // Ensure the executive is detached from the terminal
        ensure_process_group_leader()
            .context("failed to ensure executive is detached from terminal")?;

        // Signal the parent that the executive is ready
        debug!("sending SIGUSR1 to parent {}", parent_pid);
        kill(Pid::from_raw(parent_pid), SIGUSR1)?;

        let Some(log_dir) = context.attach_ctx.flox_env_log_dir.clone() else {
            unreachable!("flox_env_log_dir must be set in activation context");
        };
        let log_file = format!("executive.{}.log", std::process::id());
        logger::init_file_logger(subsystem_verbosity, log_file, &log_dir)
            .context("failed to initialize logger")?;

        // Propagate PID field to all spans.
        // We can set this eagerly because the PID doesn't change after this entry
        // point. Re-execs of activate->executive will cross this entry point again.
        let pid = std::process::id();
        let root_span = debug_span!("flox_activations_executive", pid = pid);
        let _guard = root_span.entered();

        debug!("{self:?}");

        // TODO: Enable earlier in `flox-activations` rather than just when detached?
        // TODO: Re-enable sentry after fixing OpenSSL dependency issues
        // let disable_metrics = env::var(FLOX_DISABLE_METRICS_VAR).is_ok();
        // let _sentry_guard = (!disable_metrics).then(sentry::init_sentry);

        // TODO: Use types to group the mutually optional fields for containers.
        if !context.run_monitoring_loop {
            debug!("monitoring loop disabled, exiting executive");
            return Ok(());
        }
        let Some(socket_path) = context.attach_ctx.flox_services_socket.clone() else {
            unreachable!("flox_services_socket must be set in activation context");
        };

        spawn_heartbeat_log();
        spawn_logs_gc_threads(&log_dir);

        debug!("starting monitoring loop");
        run_monitoring_loop(
            context.attach_ctx,
            socket_path,
            log_dir,
            subsystem_verbosity.unwrap_or(0),
        )
    }
}

/// Ensures the executive is detached from the terminal by becoming a process group leader.
///
/// We want to make sure that the executive is detached from the terminal in case it sends
/// any signals to the activation. A terminal sends signals to all processes in a process group,
/// and we want to make sure that the executive is in its own process group to avoid receiving any
/// signals intended for the shell.
///
/// From local testing I haven't been able to deliver signals to the executive by sending signals to
/// the activation, so this is more of a "just in case" measure.
fn ensure_process_group_leader() -> Result<(), anyhow::Error> {
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

/// Event-driven monitoring loop that watches activation processes and performs cleanup.
///
/// Uses waitpid_any for efficient process monitoring instead of polling.
#[instrument("monitoring", err(Debug), skip_all)]
fn run_monitoring_loop(
    // AttachCtx from when the Executive was started.
    // Does NOT represent the most recent attach.
    initial_attach_ctx: AttachCtx,
    socket_path: PathBuf,
    log_dir: PathBuf,
    subsystem_verbosity: u32,
) -> Result<()> {
    let dot_flox_path = initial_attach_ctx.dot_flox_path.clone();
    let runtime_dir: PathBuf = initial_attach_ctx.flox_runtime_dir.clone().into();
    let state_json_path = state_json_path(&runtime_dir, &dot_flox_path);

    debug!(
        socket = traceable_path(&socket_path),
        exists = &socket_path.exists(),
        "checked socket"
    );

    // Read initial state and start monitoring existing PIDs
    let (activations_json, lock) = read_activations_json(&state_json_path)?;
    let Some(activations) = activations_json else {
        bail!("executive shouldn't be running when state.json doesn't exist");
    };

    // Create the coordinator and start monitoring existing PIDs
    let coordinator = PidMonitorCoordinator::new();
    for (pid, expiration) in activations.all_attached_pids_with_expiration() {
        coordinator.start_monitoring(pid, expiration);
    }
    drop(lock); // Release lock after reading

    // Start file watcher for state.json changes (spawns new PID watchers directly)
    let _watcher = coordinator
        .start_state_watcher(state_json_path.clone())
        .context("failed to start state watcher")?;

    // Start signal handler thread
    let _signal_handler = coordinator
        .start_signal_handler()
        .context("failed to start signal handler")?;

    debug!("entering event-driven monitoring loop");

    run_event_loop(
        coordinator,
        initial_attach_ctx,
        socket_path,
        log_dir,
        subsystem_verbosity,
    )
}

/// Internal event loop that processes events from the coordinator.
/// Separated for testability.
fn run_event_loop(
    coordinator: PidMonitorCoordinator,
    initial_attach_ctx: AttachCtx,
    socket_path: PathBuf,
    log_dir: PathBuf,
    subsystem_verbosity: u32,
) -> Result<()> {
    let dot_flox_path = initial_attach_ctx.dot_flox_path.clone();
    let runtime_dir: PathBuf = initial_attach_ctx.flox_runtime_dir.clone().into();
    let state_json_path = state_json_path(&runtime_dir, &dot_flox_path);

    loop {
        match coordinator.receiver.recv() {
            Ok(PidEvent::ProcessExited { pid }) => {
                debug!(pid, "received ProcessExited event");

                // Re-read state and check if we need to clean up
                let (activations_json, lock) = read_activations_json(&state_json_path)?;
                let Some(mut activations) = activations_json else {
                    bail!("executive shouldn't be running when state.json doesn't exist");
                };

                // Detach the PID (idempotent if already removed)
                activations.detach(pid);

                // Check if all PIDs have terminated
                if activations.attached_pids_is_empty() {
                    info!("running cleanup after all PIDs terminated");
                    cleanup_all(
                        (activations, lock),
                        &socket_path,
                        activation_state_dir_path(&runtime_dir, &dot_flox_path),
                    )
                    .context("cleanup failed")?;
                    return Ok(());
                }

                // Clean up empty start IDs and write state
                let now = time::OffsetDateTime::now_utc();
                let (empty_start_ids, _) =
                    activations.cleanup_pids(flox_core::proc_status::pid_is_running, now);
                for start_id in empty_start_ids {
                    if let Ok(state_dir) = start_id.state_dir_path(&runtime_dir, &dot_flox_path) {
                        debug!(?state_dir, "removing empty activation state dir");
                        let _ = std::fs::remove_dir_all(state_dir);
                    }
                }

                write_activations_json(&activations, &state_json_path, lock)?;
            },
            Ok(PidEvent::TerminationSignal) => {
                // If we get a SIGINT/SIGTERM/SIGQUIT we leave behind the activation in the registry,
                // but there's not much we can do about that because we don't know who sent us one of those
                // signals or why.
                bail!("received stop signal, exiting without cleanup");
            },
            Ok(PidEvent::SigChld) => {
                reap_orphaned_children();
            },
            Ok(PidEvent::StartServices) => {
                debug!("Received SIGUSR1, starting process-compose");
                let (activations_json, lock) = read_activations_json(&state_json_path)?;
                let Some(activations) = activations_json else {
                    bail!("executive shouldn't be running when state.json doesn't exist");
                };

                match handle_start_services_signal(
                    (activations, lock),
                    &socket_path,
                    &log_dir,
                    subsystem_verbosity,
                    &initial_attach_ctx,
                ) {
                    Ok(Some((activations, lock))) => {
                        write_activations_json(&activations, &state_json_path, lock)?;
                    },
                    Ok(None) => {},
                    Err(err) => {
                        error!(%err, "failed to handle start services signal");
                    },
                }
            },
            Err(_) => {
                bail!("event channel disconnected");
            },
        }
    }
}

/// Handle the SIGUSR1 signal to start process-compose.
///
/// Return:
/// - `Some(LockedActivationState)` if state was modified and needs to be written
/// - `None` if there were no changes and the lock was dropped
fn handle_start_services_signal(
    locked_activations: LockedActivationState,
    socket_path: &Path,
    log_dir: &Path,
    subsystem_verbosity: u32,
    attach_ctx: &AttachCtx,
) -> Result<Option<LockedActivationState>> {
    let (mut activations, lock) = locked_activations;

    // There's nothing we can do if another "start" has occurred in the time it
    // took us to receive and process the signal. `flox-activations activate`
    // may timeout and present an error to the user.
    let Some(ready_start_id) = activations.ready_start_id().cloned() else {
        info!(
            reason = "no currently ready activation to attach",
            "skipping process-compose start"
        );
        return Ok(None);
    };

    // `flox-activations activate` ensures that `process-compose` is stopped
    // (and the socket removed) before signaling a restart.
    if socket_path.exists() {
        info!(reason = "already running", "skipping process-compose start");
        return Ok(None);
    }

    start_process_compose_no_services(
        socket_path,
        log_dir,
        subsystem_verbosity,
        attach_ctx,
        &ready_start_id,
    )?;

    activations.set_current_process_compose_start_id(ready_start_id);

    Ok(Some((activations, lock)))
}

/// Shutdown `process-compose` if running and remove all activation state.
/// To be called when there are no longer any PIDs attached.
fn cleanup_all(
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

#[cfg(test)]
mod test {
    use flox_core::activate::mode::ActivateMode;
    use flox_core::activations::test_helpers::write_activation_state;
    use flox_core::activations::{ActivationState, StartOrAttachResult};

    use super::pid_monitor::PidMonitorCoordinator;
    use super::watcher::test::{start_process, stop_process};
    use super::*;

    /// Create a minimal AttachCtx for testing.
    /// The actual values don't matter since tests don't trigger SIGUSR1.
    fn test_attach_ctx(dot_flox_path: &Path, runtime_dir: &Path, flox_env: &str) -> AttachCtx {
        AttachCtx {
            dot_flox_path: dot_flox_path.to_path_buf(),
            env: flox_env.to_string(),
            env_project: None,
            env_cache: dot_flox_path.join("cache"),
            env_description: "test".to_string(),
            flox_active_environments: "".to_string(),
            flox_env_log_dir: None,
            prompt_color_1: "".to_string(),
            prompt_color_2: "".to_string(),
            flox_prompt_environments: "".to_string(),
            set_prompt: false,
            flox_runtime_dir: runtime_dir.to_string_lossy().to_string(),
            flox_env_cuda_detection: "".to_string(),
            flox_services_socket: None,
            services_to_start: Vec::new(),
            interpreter_path: PathBuf::from("/nix/store/fake"),
        }
    }

    #[test]
    fn monitoring_loop_removes_state_on_cleanup() {
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

        let attach_ctx = test_attach_ctx(&dot_flox_path, runtime_dir, &flox_env.to_string_lossy());

        // Create coordinator and start monitoring the PID (which is already dead)
        let coordinator = PidMonitorCoordinator::new();
        coordinator.start_monitoring(pid, None);

        run_event_loop(
            coordinator,
            attach_ctx,
            PathBuf::from("/does_not_exist"),
            PathBuf::from("/tmp/test_log_dir"),
            0,
        )
        .unwrap();

        // Verify state directory is completely removed after cleanup
        assert!(
            !activation_state_directory.exists(),
            "state directory should be removed after cleanup"
        );
    }

    #[test]
    fn monitoring_loop_bails_on_termination_signal() {
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
            "state directory should exist before monitoring loop"
        );

        // Create coordinator and immediately send termination signal
        let coordinator = PidMonitorCoordinator::new();
        coordinator.sender().send(PidEvent::TerminationSignal).unwrap();

        let attach_ctx = test_attach_ctx(&dot_flox_path, runtime_dir, &flox_env.to_string_lossy());

        let result = run_event_loop(
            coordinator,
            attach_ctx,
            PathBuf::from("/does_not_exist"),
            PathBuf::from("/tmp/test_log_dir"),
            0,
        );

        // Verify the loop exited with the expected error
        let err = result.expect_err("should return error on termination signal");
        assert!(
            err.to_string().contains("received stop signal"),
            "error should indicate stop signal was received: {err}"
        );

        // Verify cleanup did NOT occur - state directory should still exist
        assert!(
            activation_state_directory.exists(),
            "state directory should NOT be removed when exiting due to termination signal"
        );

        // Clean up the process
        stop_process(proc);
    }
}
