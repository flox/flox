use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, LazyLock};
use std::{env, fs};

use anyhow::{Context, Result, bail};
use flox_core::activations::{
    activation_state_dir_path,
    activations_json_path,
    read_activations_json,
    write_activations_json,
};
use flox_core::traceable_path;
use logger::{spawn_heartbeat_log, spawn_logs_gc_threads};
use nix::libc::{SIGINT, SIGQUIT, SIGTERM, SIGUSR1};
use nix::unistd::{getpgid, getpid, setsid};
use process::{LockedActivations, PidWatcher, WaitResult};
use tracing::{debug, error, info, instrument};

use crate::process::Watcher;

mod logger;
mod process;
mod sentry;
pub use sentry::init_sentry;

type Error = anyhow::Error;

pub static PROCESS_COMPOSE_BIN: LazyLock<String> = LazyLock::new(|| {
    env::var("PROCESS_COMPOSE_BIN").unwrap_or(env!("PROCESS_COMPOSE_BIN").to_string())
});

#[derive(Debug, Clone)]
pub struct Cli {
    /// The path to the Flox environment
    pub flox_env: PathBuf,

    /// The path to the runtime directory keeping activation data
    pub runtime_dir: PathBuf,

    /// The activation ID to monitor
    pub activation_id: String,

    /// The path to the process-compose socket
    pub socket_path: PathBuf,

    /// The directory to store and garbage collect logs
    pub log_dir: PathBuf,

    /// Disable metric reporting
    pub disable_metrics: bool,
}

#[instrument("watchdog",
    err(Debug),
    skip_all,
    fields(pid = tracing::field::Empty,
        registry = tracing::field::Empty,
        dot_flox_hash = tracing::field::Empty,
        socket = tracing::field::Empty,
        log_dir = tracing::field::Empty))]
pub fn run(args: Cli) -> Result<(), Error> {
    let span = tracing::Span::current();
    span.record("flox_env", traceable_path(&args.flox_env));
    span.record("runtime_dir", traceable_path(&args.runtime_dir));
    span.record("id", &args.activation_id);
    span.record("socket", traceable_path(&args.socket_path));
    span.record("log_dir", traceable_path(&args.log_dir));
    debug!("starting");

    ensure_process_group_leader().context("failed to ensure watchdog is detached from terminal")?;

    // Set the signal handler
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

    run_inner(args, should_terminate, should_clean_up)
}

/// Function to be used for unit tests that doesn't do weird process stuff
fn run_inner(
    args: Cli,
    should_terminate: Arc<AtomicBool>,
    should_clean_up: Arc<AtomicBool>,
) -> Result<(), Error> {
    let activations_json_path = activations_json_path(&args.runtime_dir, &args.flox_env);

    let mut watcher = PidWatcher::new(
        activations_json_path.clone(),
        args.activation_id.clone(),
        should_terminate,
        should_clean_up,
    );

    debug!(
        path = traceable_path(&args.socket_path),
        exists = &args.socket_path.exists(),
        "checked socket"
    );

    info!(
        this_pid = nix::unistd::getpid().as_raw(),
        target_activation_id = args.activation_id,
        "watchdog is on duty"
    );
    spawn_heartbeat_log();
    spawn_logs_gc_threads(args.log_dir);

    debug!("waiting for termination");

    let activation_state_dir =
        activation_state_dir_path(&args.runtime_dir, &args.flox_env, &args.activation_id)?;

    match watcher.wait_for_termination() {
        Ok(WaitResult::CleanUp(locked_activations)) => {
            // Exit
            info!("running cleanup after all PIDs terminated");
            cleanup(
                locked_activations,
                &args.socket_path,
                &activations_json_path,
                &activation_state_dir,
                &args.activation_id,
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
            let (activations_json, lock) = read_activations_json(&activations_json_path)?;
            let Some(activations_json) = activations_json else {
                bail!("watchdog shouldn't be running when activations.json doesn't exist");
            };
            let activations = activations_json.check_version()?;
            let _ = cleanup(
                (activations, lock),
                &args.socket_path,
                &activations_json_path,
                &activation_state_dir,
                &args.activation_id,
            );
            bail!(err.context("failed while waiting for termination"))
        },
    }

    Ok(())
}

// If the activation for a watchdog gets removed from the registry as stale by a different watchdog,
// multiple watchdogs could perform cleanup.
// The following can be run multiple times without issue.
fn cleanup(
    locked_activations: LockedActivations,
    socket_path: impl AsRef<Path>,
    activations_json_path: impl AsRef<Path>,
    activation_state_dir_path: impl AsRef<Path>,
    activation_id: impl AsRef<str>,
) -> Result<()> {
    debug!("running cleanup");

    let (mut activations_json, lock) = locked_activations;
    activations_json.remove_activation(activation_id);

    // Even if this activation has no more attached PIDs, there may be other
    // activations for a different build of the same environment
    if activations_json.is_empty() {
        let socket_path = socket_path.as_ref();
        if socket_path.exists() {
            if let Err(err) = process_compose_down(socket_path) {
                error!(%err, "failed to run process-compose shutdown command");
            }
        } else {
            debug!(reason = "no socket", "did not shut down process-compose");
        }
    }

    fs::remove_dir_all(activation_state_dir_path)
        .context("couldn't remove activations state dir")?;

    // We want to hold the lock until
    // - services are cleaned up
    // - activation state dir is removed, otherwise the removal could occur
    //   after a newly started activation has already put files in activation
    //   state dir
    write_activations_json(&activations_json, activations_json_path, lock)?;

    debug!("finished cleanup");

    Ok(())
}

/// Shuts down process-compose by running `process-compose down` via the unix socket.
///
/// This is a variation of `providers::services::process_compose_down` to avoid
/// the dependency on `flox-rust-sdk`.
fn process_compose_down(socket_path: impl AsRef<Path>) -> Result<()> {
    let mut cmd = Command::new(&*PROCESS_COMPOSE_BIN);
    cmd.arg("down");
    cmd.arg("--unix-socket");
    cmd.arg(socket_path.as_ref());
    cmd.env("NO_COLOR", "1");

    debug!(
        command = format!(
            "{} down --unix-socket {}",
            *PROCESS_COMPOSE_BIN,
            socket_path.as_ref().display()
        ),
        "running process-compose down"
    );

    let output = cmd
        .output()
        .context("failed to execute process-compose down")?;

    output.status.success().then_some(()).ok_or_else(|| {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::anyhow!("process-compose down failed: {}", stderr)
    })
}

/// We want to make sure that the watchdog is detached from the terminal in case it sends
/// any signals to the activation. A terminal sends signals to all processes in a process group,
/// and we want to make sure that the watchdog is in its own process group to avoid receiving any
/// signals intended for the shell.
///
/// From local testing I haven't been able to deliver signals to the watchdog by sending signals to
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
    use flox_activations::cli::{SetReadyArgs, StartOrAttachArgs};
    use process::test::{shutdown_flags, start_process, stop_process};

    use super::*;

    #[test]
    fn cleanup_removes_activation() {
        let temp_dir = tempfile::tempdir().unwrap();
        let runtime_dir = temp_dir.path();
        let log_dir = temp_dir.path();
        let flox_env = PathBuf::from("flox_env");
        let store_path = "store_path".to_string();

        let proc = start_process();
        let pid = proc.id() as i32;
        let start_or_attach = StartOrAttachArgs {
            pid,
            flox_env: flox_env.clone(),
            store_path: store_path.clone(),
            runtime_dir: runtime_dir.to_path_buf(),
        };
        let activation_id = start_or_attach.handle_inner().unwrap().activation_id;
        let set_ready = SetReadyArgs {
            id: activation_id.clone(),
            flox_env: flox_env.clone(),
            runtime_dir: runtime_dir.to_path_buf(),
        };
        set_ready.handle().unwrap();

        let activations_json_path = activations_json_path(runtime_dir, &flox_env);

        let activations_json = read_activations_json(&activations_json_path)
            .unwrap()
            .0
            .unwrap()
            .check_version()
            .unwrap();
        assert!(!activations_json.is_empty());

        stop_process(proc);

        let cli = Cli {
            flox_env,
            runtime_dir: runtime_dir.to_path_buf(),
            activation_id,
            socket_path: PathBuf::from("/does_not_exist"),
            log_dir: log_dir.to_path_buf(),
            disable_metrics: true,
        };

        let (terminate_flag, cleanup_flag) = shutdown_flags();
        run_inner(cli, terminate_flag, cleanup_flag).unwrap();

        let activations_json = read_activations_json(&activations_json_path)
            .unwrap()
            .0
            .unwrap()
            .check_version()
            .unwrap();
        assert!(activations_json.is_empty());
    }
}
