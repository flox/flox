use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::{Context, Result, bail};
use clap::Args;
use flox_core::activate::context::ActivateCtx;
use flox_core::activations::{activation_state_dir_path, read_activations_json, state_json_path};
use flox_core::traceable_path;
use log_gc::{spawn_heartbeat_log, spawn_logs_gc_threads};
use nix::libc::{SIGCHLD, SIGINT, SIGQUIT, SIGTERM};
use nix::sys::signal::Signal::SIGUSR1;
use nix::sys::signal::kill;
use nix::unistd::{Pid, getpgid, getpid, setsid};
use serde::{Deserialize, Serialize};
use signal_hook::iterator::Signals;
use tracing::{debug, debug_span, error, info, instrument};
use watcher::{LockedActivationState, PidWatcher, WaitResult, Watcher};

use crate::cli::activate::NO_REMOVE_ACTIVATION_FILES;
use crate::logger;
use crate::process_compose::process_compose_down;

mod log_gc;
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

        let Some(log_dir) = &context.attach_ctx.flox_env_log_dir else {
            unreachable!("flox_env_log_dir must be set in activation context");
        };
        let log_file = format!("executive.{}.log", std::process::id());
        logger::init_file_logger(subsystem_verbosity, log_file, log_dir)
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
        let Some(socket_path) = &context.attach_ctx.flox_services_socket else {
            unreachable!("flox_services_socket must be set in activation context");
        };

        spawn_heartbeat_log();
        spawn_logs_gc_threads(log_dir);

        // Set up signal handlers just before entering the monitoring loop.
        // Doing this too early could prevent the executive from being killed if
        // it gets stuck.
        let should_clean_up = Arc::new(AtomicBool::new(false));
        signal_hook::flag::register(nix::libc::SIGUSR1, Arc::clone(&should_clean_up))
            .context("failed to set SIGUSR1 signal handler")?;
        let should_terminate = Arc::new(AtomicBool::new(false));
        signal_hook::flag::register(SIGINT, Arc::clone(&should_terminate))
            .context("failed to set SIGINT signal handler")?;
        signal_hook::flag::register(SIGTERM, Arc::clone(&should_terminate))
            .context("failed to set SIGTERM signal handler")?;
        signal_hook::flag::register(SIGQUIT, Arc::clone(&should_terminate))
            .context("failed to set SIGQUIT signal handler")?;
        // This compliments the SubreaperGuard setup above.
        // WARNING: You cannot reliably use Command::wait after we've entered the
        // monitoring loop, including concurrent threads like GCing logs, because
        // children will be reaped automatically.
        let should_reap = Signals::new([SIGCHLD])?;

        debug!("starting monitoring loop");
        run_monitoring_loop(
            context.attach_ctx.dot_flox_path,
            context.attach_ctx.flox_runtime_dir.into(),
            socket_path.into(),
            should_terminate,
            should_clean_up,
            should_reap,
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

/// Monitoring loop that watches activation processes and performs cleanup.
#[instrument("monitoring", err(Debug), skip_all)]
fn run_monitoring_loop(
    dot_flox_path: PathBuf,
    runtime_dir: PathBuf,
    socket_path: PathBuf,
    should_terminate: Arc<AtomicBool>,
    should_clean_up: Arc<AtomicBool>,
    should_reap: Signals,
) -> Result<()> {
    let state_json_path = state_json_path(&runtime_dir, &dot_flox_path);

    let mut watcher = PidWatcher::new(
        state_json_path.clone(),
        dot_flox_path.clone(),
        runtime_dir.clone(),
        should_terminate,
        should_clean_up,
        should_reap,
    );

    debug!(
        socket = traceable_path(&socket_path),
        exists = &socket_path.exists(),
        "checked socket"
    );

    match watcher.wait_for_termination() {
        Ok(WaitResult::CleanUp(locked_activations)) => {
            // Exit
            info!("running cleanup after all PIDs terminated");
            cleanup(
                locked_activations,
                &socket_path,
                activation_state_dir_path(&runtime_dir, &dot_flox_path),
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
                &socket_path,
                activation_state_dir_path(&runtime_dir, &dot_flox_path),
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

#[cfg(test)]
mod test {
    use flox_core::activate::mode::ActivateMode;
    use flox_core::activations::test_helpers::write_activation_state;
    use flox_core::activations::{ActivationState, StartOrAttachResult};

    use super::watcher::test::{shutdown_flags, start_process, stop_process};
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

        let (terminate_flag, cleanup_flag, reap_flag) = shutdown_flags();
        run_monitoring_loop(
            dot_flox_path.clone(),
            runtime_dir.to_path_buf(),
            PathBuf::from("/does_not_exist"),
            terminate_flag,
            cleanup_flag,
            reap_flag,
        )
        .unwrap();

        // Verify state directory is completely removed after cleanup
        assert!(
            !activation_state_directory.exists(),
            "state directory should be removed after cleanup"
        );
    }
}
