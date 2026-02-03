use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::Args;
use flox_core::activate::context::{AttachCtx, AttachProjectCtx};
use flox_core::activate::vars::FLOX_EXECUTIVE_VERBOSITY_VAR;
use flox_core::activations::{read_activations_json, state_json_path, write_activations_json};
use flox_core::traceable_path;
use log_gc::{spawn_heartbeat_log, spawn_logs_gc_threads};
use nix::libc::{SIGCHLD, SIGINT, SIGQUIT, SIGTERM};
use nix::sys::signal::Signal::SIGUSR1;
use nix::sys::signal::kill;
use nix::unistd::{Pid, getpgid, getpid, setsid};
use reaper::reap_orphaned_children;
use serde::{Deserialize, Serialize};
use signal_hook::iterator::Signals;
use tracing::{debug, debug_span, error, info, instrument};
use watcher::{LockedActivationState, PidWatcher};

use crate::cli::activate::NO_REMOVE_ACTIVATION_FILES;
use crate::logger;
use crate::process_compose::{process_compose_down, start_process_compose_no_services};

mod log_gc;
mod reaper;
mod watcher;
// TODO: Re-enable sentry after fixing OpenSSL dependency issues
// mod sentry;

#[cfg(target_os = "linux")]
use reaper::linux::SubreaperGuard;

/// How long to wait between monitoring loop iterations.
const MONITORING_LOOP_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutiveCtx {
    pub attach_ctx: AttachCtx,
    pub project_ctx: AttachProjectCtx,
    pub activation_state_dir: std::path::PathBuf,
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
    pub fn handle(self) -> Result<(), anyhow::Error> {
        let contents = fs::read_to_string(&self.executive_ctx)?;
        let ExecutiveCtx {
            attach_ctx,
            project_ctx,
            activation_state_dir,
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

        // Set up signal handlers early. All signals registered together.
        let signals = SignalHandlers::new()?;

        // Signal the parent that the executive is ready
        debug!("sending SIGUSR1 to parent {}", parent_pid);
        kill(Pid::from_raw(parent_pid), SIGUSR1)?;

        let log_dir = project_ctx.flox_env_log_dir.clone();
        let log_file = format!("executive.{}.log", std::process::id());
        // Read verbosity from dedicated executive variable, not `activate -v`
        // Only takes numeric values like its `FLOX_ACTIVATIONS_VERBOSITY` counterpart.
        let subsystem_verbosity = std::env::var(FLOX_EXECUTIVE_VERBOSITY_VAR)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
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

        spawn_heartbeat_log();
        spawn_logs_gc_threads(&log_dir);

        debug!("starting monitoring loop");
        run_monitoring_loop(
            attach_ctx,
            project_ctx,
            activation_state_dir,
            signals,
            subsystem_verbosity,
        )?;
        Ok(())
    }
}

/// Handles signal registration and checking for the executive process.
///
/// All signals are registered together early in `handle()`. SIGKILL is always
/// available as a fallback if the executive gets stuck during startup.
#[derive(Debug)]
pub struct SignalHandlers {
    should_terminate: Arc<AtomicBool>,
    should_start_services: Arc<AtomicBool>,
    should_reap: Signals,
}

impl SignalHandlers {
    /// Register all signal handlers.
    pub fn new() -> Result<Self> {
        let should_terminate = Arc::new(AtomicBool::new(false));
        let should_start_services = Arc::new(AtomicBool::new(false));
        signal_hook::flag::register(SIGINT, Arc::clone(&should_terminate))
            .context("failed to set SIGINT signal handler")?;
        signal_hook::flag::register(SIGTERM, Arc::clone(&should_terminate))
            .context("failed to set SIGTERM signal handler")?;
        signal_hook::flag::register(SIGQUIT, Arc::clone(&should_terminate))
            .context("failed to set SIGQUIT signal handler")?;
        signal_hook::flag::register(nix::libc::SIGUSR1, Arc::clone(&should_start_services))
            .context("failed to set SIGUSR1 signal handler")?;
        // This complements the SubreaperGuard setup.
        // WARNING: You cannot reliably use Command::wait after SignalHandlers is
        // created, including concurrent threads like GCing logs, because children
        // will be reaped automatically.
        let should_reap = Signals::new([SIGCHLD])?;
        Ok(Self {
            should_terminate,
            should_start_services,
            should_reap,
        })
    }

    /// Check if a termination signal has been received.
    pub fn should_terminate(&self) -> bool {
        self.should_terminate.load(Ordering::SeqCst)
    }

    /// Check if SIGUSR1 was received (start services signal).
    /// Atomically clears the flag after reading.
    pub fn should_start_services(&self) -> bool {
        self.should_start_services.swap(false, Ordering::SeqCst)
    }

    /// Reap any children that have terminated since the last check.
    pub fn reap_pending_children(&mut self) {
        for _ in self.should_reap.pending() {
            reap_orphaned_children();
        }
    }

    /// Create SignalHandlers for testing without registering real signal handlers.
    #[cfg(test)]
    pub fn new_for_test() -> Result<Self> {
        const NO_SIGNALS: &[i32] = &[];
        Ok(Self {
            should_terminate: Arc::new(AtomicBool::new(false)),
            should_start_services: Arc::new(AtomicBool::new(false)),
            should_reap: Signals::new(NO_SIGNALS).context("failed to create Signals")?,
        })
    }

    /// Trigger termination flag for testing.
    #[cfg(test)]
    pub fn trigger_termination(&self) {
        self.should_terminate.store(true, Ordering::SeqCst);
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

/// Monitoring loop that watches activation processes and performs cleanup.
#[instrument("monitoring", err(Debug), skip_all)]
fn run_monitoring_loop(
    // AttachCtx from when the Executive was started.
    // Does NOT represent the most recent attach.
    initial_attach_ctx: AttachCtx,
    project_ctx: AttachProjectCtx,
    activation_state_dir: PathBuf,
    mut signals: SignalHandlers,
    subsystem_verbosity: u32,
) -> Result<()> {
    let state_json_path = state_json_path(&activation_state_dir);

    let mut watcher = PidWatcher::new(state_json_path.clone(), activation_state_dir.clone());

    let process_compose_bin = project_ctx.process_compose_bin.to_path_buf();
    let socket_path = project_ctx.flox_services_socket.to_path_buf();
    debug!(
        socket = traceable_path(&socket_path),
        exists = &socket_path.exists(),
        "checked socket"
    );

    loop {
        // Check for terminated PIDs and clean up state
        match watcher.cleanup_pids() {
            Ok(None) => {
                // Still have active PIDs, continue monitoring
            },
            Ok(Some(locked_activations)) => {
                info!("running cleanup after all PIDs terminated");
                cleanup_all(
                    locked_activations,
                    &process_compose_bin,
                    &socket_path,
                    &activation_state_dir,
                )
                .context("cleanup failed")?;
                return Ok(());
            },
            Err(err) => {
                info!("running cleanup after error");
                let (activations_json, lock) = read_activations_json(&state_json_path)?;
                let Some(activations) = activations_json else {
                    bail!("executive shouldn't be running when state.json doesn't exist");
                };
                let _ = cleanup_all(
                    (activations, lock),
                    &process_compose_bin,
                    &socket_path,
                    &activation_state_dir,
                );
                bail!(err.context("failed while waiting for termination"))
            },
        }

        // Check for termination signals
        if signals.should_terminate() {
            // If we get a SIGINT/SIGTERM/SIGQUIT we leave behind the activation in the registry,
            // but there's not much we can do about that because we don't know who sent us one of those
            // signals or why.
            bail!("received stop signal, exiting without cleanup");
        }

        // Check for SIGUSR1 (start services signal) after cleanup and termination checks
        if signals.should_start_services() {
            debug!("Received SIGUSR1, starting process-compose");
            let (activations_json, lock) = read_activations_json(&state_json_path)?;
            let Some(activations) = activations_json else {
                bail!("executive shouldn't be running when state.json doesn't exist");
            };

            match handle_start_services_signal(
                (activations, lock),
                subsystem_verbosity,
                &initial_attach_ctx,
                &project_ctx,
                &activation_state_dir,
            ) {
                Ok(Some((activations, lock))) => {
                    write_activations_json(&activations, &state_json_path, lock)?;
                },
                Ok(None) => {},
                Err(err) => {
                    error!(%err, "failed to handle start services signal");
                },
            }
        }

        // Reap any orphaned children
        signals.reap_pending_children();

        std::thread::sleep(MONITORING_LOOP_INTERVAL);
    }
}

/// Handle the SIGUSR1 signal to start process-compose.
///
/// Return:
/// - `Some(LockedActivationState)` if state was modified and needs to be written
/// - `None` if there were no changes and the lock was dropped
fn handle_start_services_signal(
    locked_activations: LockedActivationState,
    subsystem_verbosity: u32,
    attach_ctx: &AttachCtx,
    project_ctx: &AttachProjectCtx,
    activation_state_dir: &Path,
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
    if project_ctx.flox_services_socket.exists() {
        info!(reason = "already running", "skipping process-compose start");
        return Ok(None);
    }

    start_process_compose_no_services(
        subsystem_verbosity,
        attach_ctx,
        project_ctx,
        &ready_start_id,
        activation_state_dir,
    )?;

    activations.set_current_process_compose_start_id(ready_start_id);

    Ok(Some((activations, lock)))
}

/// Shutdown `process-compose` if running and remove all activation state.
/// To be called when there are no longer any PIDs attached.
fn cleanup_all(
    locked_activations: LockedActivationState,
    process_compose_bin: &Path,
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
        if let Err(err) = process_compose_down(process_compose_bin, socket_path) {
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
    use flox_core::activations::{ActivationState, StartOrAttachResult, activation_state_dir_path};

    use super::watcher::test::{start_process, stop_process};
    use super::*;

    /// Create minimal context for testing.
    /// The actual values don't matter since tests don't trigger SIGUSR1.
    fn test_context(
        dot_flox_path: &Path,
        runtime_dir: &Path,
        flox_env: &str,
    ) -> (AttachCtx, AttachProjectCtx) {
        let attach = AttachCtx {
            env: flox_env.to_string(),
            env_description: "test".to_string(),
            env_cache: dot_flox_path.join("cache"),
            flox_runtime_dir: runtime_dir.to_string_lossy().to_string(),
            interpreter_path: PathBuf::from("/nix/store/fake"),
            prompt_color_1: "".to_string(),
            prompt_color_2: "".to_string(),
            flox_prompt_environments: "".to_string(),
            set_prompt: false,
            flox_env_cuda_detection: "".to_string(),
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
        let mut state =
            ActivationState::new(&ActivateMode::default(), Some(&dot_flox_path), &flox_env);
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

        let (attach, project) =
            test_context(&dot_flox_path, runtime_dir, &flox_env.to_string_lossy());

        run_monitoring_loop(
            attach,
            project,
            activation_state_directory.clone(),
            SignalHandlers::new_for_test().unwrap(),
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
        let mut state =
            ActivationState::new(&ActivateMode::default(), Some(&dot_flox_path), &flox_env);
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

        // Create SignalHandlers and trigger termination before starting the loop
        let signals = SignalHandlers::new_for_test().unwrap();
        signals.trigger_termination();

        let (attach, project) =
            test_context(&dot_flox_path, runtime_dir, &flox_env.to_string_lossy());

        let result = run_monitoring_loop(
            attach,
            project,
            activation_state_directory.clone(),
            signals,
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
