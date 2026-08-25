//! Start logic for activations.
//!
//! This module contains the core logic for starting new activations,
//! including spawning the executive process, running hooks, and
//! managing process-compose for services.

use std::fs::DirBuilder;
use std::os::unix::fs::DirBuilderExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use flox_core::activate::context::{ActivateCtx, AttachCtx, AttachProjectCtx};
use flox_core::activate::vars::FLOX_ACTIVATIONS_BIN;
use flox_core::activations::{
    ActivationState,
    StartIdentifier,
    StartOrAttachResult,
    read_activations_json,
    state_json_path,
    write_activations_json,
};
use fslock::LockFile;
use indoc::{formatdoc, indoc};
use nix::sys::signal::{Signal, kill};
use nix::sys::wait::{WaitPidFlag, WaitStatus, waitpid};
use nix::unistd::{Pid, getpid};
use signal_hook::consts::{SIGCHLD, SIGUSR1};
use signal_hook::iterator::Signals;
use tracing::{debug, error};
use uuid::Uuid;

use crate::attach_diff::assemble_activate_command;
use crate::cli::executive::ExecutiveCtx;
use crate::process_compose::{
    latest_services_log,
    log_tail,
    process_compose_down,
    start_services_via_socket,
    wait_for_socket_ready,
    wait_for_socket_removed,
};
use crate::vars_from_env::VarsFromEnvironment;

/// Marker the activate script writes into the start state directory as its
/// last act, so that a hook which exited or exec'd out — taking the rest of
/// the script with it — is distinguishable from one that returned normally.
const ACTIVATE_COMPLETE_MARKER: &str = "complete";

/// How long to wait for a newly spawned `process-compose` to answer on its
/// socket before giving up.
///
/// Only reached when something is wrong: the wait ends as soon as the socket
/// answers, which is a few hundred milliseconds on an idle machine.
const DEFAULT_ACTIVATE_TIMEOUT: Duration = Duration::from_secs(2);

/// How long `process-compose` may take to unlink its socket after being told to
/// shut down.
///
/// Generous because it covers a whole shutdown sequence — stopping each service
/// and reaping it — on a machine loaded enough that all of it is slow.
const SOCKET_REMOVAL_TIMEOUT: Duration = Duration::from_secs(10);

/// How long to keep waiting on a socket that refused a connection.
///
/// Only covers the tail of someone else's shutdown, between `process-compose`
/// closing its listener and unlinking the socket. Anything longer than this is
/// a socket nobody owns, and no amount of waiting will remove it.
const SOCKET_SHUTDOWN_GRACE: Duration = Duration::from_secs(2);

/// Start a new activation because we either have a:
/// - different store path
/// - fresh state file, which could be caused by no executive
pub fn start(
    context: &ActivateCtx,
    subsystem_verbosity: u32,
    vars_from_env: &VarsFromEnvironment,
    start_id: StartIdentifier,
    activations: &mut ActivationState,
    activations_json_path: &Path,
    lock: LockFile,
) -> Result<StartOrAttachResult, anyhow::Error> {
    let start_state_dir = start_id.start_state_dir(&context.activation_state_dir)?;
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&start_state_dir)?;

    let new_executive = match context.project_ctx.as_ref() {
        // Start a new executive.
        Some(project) if !activations.executive_started() => {
            // Register signal handler BEFORE spawning executive to avoid race condition
            // where SIGUSR1 arrives before handler is registered
            let signals = Signals::new([SIGCHLD, SIGUSR1])?;
            let exec_pid = spawn_executive(
                &context.attach_ctx,
                project,
                &context.activation_state_dir,
                &start_state_dir,
                context.metrics_uuid,
            )?;
            activations.set_executive_pid(exec_pid.as_raw());
            Some((exec_pid, signals))
        },
        // Executive already started
        Some(_) => None,
        // Use own PID as an executive when there's no project, e.g. containerize.
        None => {
            let pid_self = std::process::id() as i32;
            activations.set_executive_pid(pid_self);
            None
        },
    };

    write_activations_json(activations, activations_json_path, lock)?;

    if let Some((exec_pid, signals)) = new_executive {
        wait_for_executive(exec_pid, signals)?;
    }

    let mut start_command = assemble_activate_command(
        context,
        subsystem_verbosity,
        vars_from_env.clone(),
        &start_state_dir,
    );
    debug!("spawning activate script: {:?}", start_command);
    let status = start_command.spawn()?.wait()?;
    if !status.success() {
        // hook.on-activate may have already printed to stderr
        bail!("Running hook.on-activate failed");
    }

    if !start_state_dir.join(ACTIVATE_COMPLETE_MARKER).exists() {
        bail!(indoc! {"
            The hook.on-activate script did not complete normally.

            Review your script for the use of:
            - 'exit' commands, which should be replaced with 'return'
            - 'exec' commands, which should be run in a subshell: '(exec command)'"});
    }

    // Re-acquire lock to mark ready
    let (activations_opt, lock) = read_activations_json(activations_json_path)?;
    let mut activations = activations_opt.expect("state.json should exist");
    activations.set_ready(&start_id);
    write_activations_json(&activations, activations_json_path, lock)?;

    Ok(StartOrAttachResult::Start { start_id })
}

/// Start services with a new process-compose instance.
///
/// The CLI has already decided that a new process-compose is needed.
/// This function starts process-compose and then starts the specified services.
pub fn start_services_with_new_process_compose(
    activation_state_dir: &Path,
    project: &AttachProjectCtx,
) -> Result<(), anyhow::Error> {
    let activations_json_path = state_json_path(activation_state_dir);
    let (activations_opt, lock) = read_activations_json(&activations_json_path)?;
    let activations = activations_opt.expect("state.json should exist");
    let executive_pid = activations.executive_pid();
    // Don't hold a lock because the executive will need it when starting `process-compose`
    drop(lock);

    debug!("starting new process-compose for services");
    signal_new_process_compose(project, executive_pid)?;
    start_services_via_socket(
        &project.process_compose_bin,
        &project.flox_services_socket,
        &project.services_to_start,
    )?;

    Ok(())
}

/// Start a new process-compose instance by signaling the executive.
fn signal_new_process_compose(
    project: &AttachProjectCtx,
    executive_pid: i32,
) -> Result<(), anyhow::Error> {
    let process_compose_bin = project.process_compose_bin.as_path();
    let socket_path = project.flox_services_socket.as_path();
    // Stop first, if running, to ensure that we wait on the socket from the new instance.
    if socket_path.exists() {
        debug!("shutting down old process-compose");
        // A shutdown that fails means nothing answered on the socket. Usually
        // that is a stale socket, which no `process-compose` is going to
        // remove, so waiting the full timeout only delays the same failure.
        // But an instance that is already shutting down stops listening before
        // it unlinks, and during that window a refused connection is not stale
        // at all, so allow for one to finish.
        if let Err(err) = process_compose_down(process_compose_bin, socket_path) {
            error!(%err, "failed to stop process-compose");
            if !wait_for_socket_removed(socket_path, SOCKET_SHUTDOWN_GRACE) {
                bail!(stale_socket_message(socket_path));
            }
        }
        // The executive treats a socket that is still on disk as an instance
        // that is already running and declines to start another one, so
        // signalling before the old socket is gone drops the start on the floor
        // and leaves nothing to wait for below.
        if !wait_for_socket_removed(socket_path, SOCKET_REMOVAL_TIMEOUT) {
            bail!(stale_socket_message(socket_path));
        }
    }

    // Note which log is the newest before asking for a start, so that the log
    // of the instance that just shut down can't be mistaken for the log of the
    // instance being waited on.
    let log_before_start = latest_services_log(&project.flox_env_log_dir);

    debug!(
        executive_pid,
        "sending SIGUSR1 to executive to start new process-compose",
    );
    kill(Pid::from_raw(executive_pid), Signal::SIGUSR1)?;

    let activation_timeout = std::env::var("_FLOX_SERVICES_ACTIVATE_TIMEOUT")
        .ok()
        .and_then(|t| t.parse().ok())
        .map(Duration::from_secs_f64)
        .unwrap_or(DEFAULT_ACTIVATE_TIMEOUT);
    let socket_ready = wait_for_socket_ready(process_compose_bin, socket_path, activation_timeout)?;
    if !socket_ready {
        bail!(socket_not_ready_message(
            &project.flox_env_log_dir,
            log_before_start.as_deref(),
            activation_timeout
        ));
    }

    Ok(())
}

/// Explain a socket that outlived the `process-compose` it belonged to.
///
/// Reached when shutting down the old instance left the socket behind, which
/// after [SOCKET_REMOVAL_TIMEOUT] means no `process-compose` is going to remove
/// it: either it is long gone and the socket is stale, or it is wedged. Nothing
/// can start while the file is there, so say what to delete.
fn stale_socket_message(socket_path: &Path) -> String {
    formatdoc! {"
        Failed to start services: a service manager socket is still in place after shutting down.
        No process is going to remove it.
        Remove {socket_path} and try again.",
        socket_path = socket_path.display(),
    }
}

/// Explain a service startup that never produced a usable socket.
///
/// The executive is what spawns `process-compose`, and it reports neither the
/// outcome nor the log path back here, so the log it wrote is the only account
/// of what went wrong. Without it the failures that reach this point — a
/// `process-compose` that died on startup, one that is merely slower than the
/// timeout, and one that was never spawned at all — are indistinguishable to
/// the reader.
///
/// `log_before_start` is the newest log from before the start was requested.
/// A newer one means an instance was spawned and its log describes this start;
/// no newer one means the executive never spawned anything, which is a
/// different problem and worth saying plainly.
fn socket_not_ready_message(
    log_dir: &Path,
    log_before_start: Option<&Path>,
    waited: Duration,
) -> String {
    let Some(log) =
        latest_services_log(log_dir).filter(|log| Some(log.as_path()) != log_before_start)
    else {
        return formatdoc! {"
            Failed to start services: no service manager was started within {waited:.1?}.
            Nothing was written to {log_dir}, so nothing tried to start.
            Try again.",
            log_dir = log_dir.display(),
        };
    };

    let Some(tail) = log_tail(&log, 10) else {
        return formatdoc! {"
            Failed to start services: the service manager did not respond within {waited:.1?}.
            It may still be starting.
            Check with 'flox services status'."};
    };

    let indented = tail
        .lines()
        .map(|line| format!("  {line}"))
        .collect::<Vec<_>>()
        .join("\n");

    formatdoc! {"
        Failed to start services: the service manager did not respond within {waited:.1?}.
        Its log ends with:
        {indented}
        The full log is at {log}.",
        log = log.display(),
    }
}

fn spawn_executive(
    attach: &AttachCtx,
    project: &AttachProjectCtx,
    activation_state_dir: &Path,
    start_state_dir: &Path,
    metrics_uuid: Option<Uuid>,
) -> Result<Pid, anyhow::Error> {
    let parent_pid = getpid();

    let executive_ctx = ExecutiveCtx {
        attach_ctx: attach.clone(),
        project_ctx: project.clone(),
        activation_state_dir: activation_state_dir.to_path_buf(),
        parent_pid: parent_pid.as_raw(),
        metrics_uuid,
    };

    let temp_file = tempfile::NamedTempFile::with_prefix_in("executive_ctx_", start_state_dir)?;
    serde_json::to_writer(&temp_file, &executive_ctx)?;
    let executive_ctx_path = temp_file.path().to_path_buf();
    temp_file.keep()?;

    // Spawn executive
    let mut executive = Command::new((*FLOX_ACTIVATIONS_BIN).clone());
    executive.args([
        "executive",
        // This is ony provided for the purpose of humans identifying the
        // process from args.
        "--dot-flox-path",
        &project.dot_flox_path.to_string_lossy(),
        "--executive-ctx",
        &executive_ctx_path.to_string_lossy(),
    ]);
    executive
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    debug!(
        "Spawning executive process to start activation: {:?}",
        executive
    );
    let child = executive.spawn()?;
    Ok(Pid::from_raw(child.id() as i32))
}

/// Wait for the executive to signal that it has started by sending SIGUSR1.
/// If the executive dies, then we error.
/// Signals should have been registered for SIGCHLD and SIGUSR1
fn wait_for_executive(child_pid: Pid, mut signals: Signals) -> Result<(), anyhow::Error> {
    debug!(
        "Awaiting SIGUSR1 from child process with PID: {}",
        child_pid
    );

    // I think the executive will always either successfully send SIGUSR1,
    // or it will exit sending SIGCHLD
    // If I'm wrong, this will loop forever
    loop {
        let pending = signals.wait();
        // We want to handle SIGUSR1 rather than SIGCHLD if both
        // are received
        // I'm not 100% confident SIGCHLD couldn't be delivered prior to
        // SIGUSR1 or SIGUSR2,
        // but I haven't seen that since switching to signals.wait() instead
        // of signals.forever()
        // If that does happen, the user would see
        // "Error: Activation process {} terminated unexpectedly"
        // which isn't a huge problem
        let signals = pending.collect::<Vec<_>>();
        // Proceed after receiving SIGUSR1
        if signals.contains(&SIGUSR1) {
            debug!(
                "Received SIGUSR1 (executive started successfully) from child process {}",
                child_pid
            );
            return Ok(());
        } else if signals.contains(&SIGCHLD) {
            // SIGCHLD can come from any child process, not just ours.
            // Use waitpid with WNOHANG to check if OUR child has exited.
            match waitpid(child_pid, Some(WaitPidFlag::WNOHANG)) {
                Ok(WaitStatus::StillAlive) => {
                    // Our child is still alive, SIGCHLD was from a different process
                    debug!(
                        "Received SIGCHLD but child {} is still alive, continuing to wait",
                        child_pid
                    );
                    continue;
                },
                Ok(status) => {
                    // Our child has exited
                    return Err(anyhow!(
                        // TODO: we should print the path to the log file
                        "Executive {} terminated unexpectedly with status: {:?}",
                        child_pid,
                        status
                    ));
                },
                Err(nix::errno::Errno::ECHILD) => {
                    // Child already reaped, this shouldn't happen but handle gracefully
                    return Err(anyhow!(
                        "Executive {} terminated unexpectedly (already reaped)",
                        child_pid
                    ));
                },
                Err(e) => {
                    // Unexpected error from waitpid
                    return Err(anyhow!(
                        "Failed to check status of executive process {}: {}",
                        child_pid,
                        e
                    ));
                },
            }
        } else {
            unreachable!("Received unexpected signal or empty iterator over signals");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A log left behind by the instance that just shut down must not be
    /// reported as the log of the start being waited on: the two failures read
    /// identically, and the leftover one describes a clean shutdown, which
    /// sends the reader looking in the wrong place.
    #[test]
    fn socket_not_ready_message_ignores_a_log_from_before_the_start() {
        let dir = tempfile::tempdir().unwrap();
        let previous = dir.path().join("services.20260825120000000000.log");
        std::fs::write(&previous, "INF Thank you for using process-compose\n").unwrap();

        let message = socket_not_ready_message(dir.path(), Some(&previous), Duration::from_secs(2));

        assert_eq!(message, formatdoc! {"
            Failed to start services: no service manager was started within 2.0s.
            Nothing was written to {log_dir}, so nothing tried to start.
            Try again.",
            log_dir = dir.path().display(),
        });
    }

    #[test]
    fn socket_not_ready_message_quotes_a_log_from_this_start() {
        let dir = tempfile::tempdir().unwrap();
        let previous = dir.path().join("services.20260825120000000000.log");
        std::fs::write(&previous, "INF Thank you for using process-compose\n").unwrap();
        let current = dir.path().join("services.20260825120001000000.log");
        std::fs::write(&current, "ERR error=\"bind: no such file or directory\"\n").unwrap();

        let message = socket_not_ready_message(dir.path(), Some(&previous), Duration::from_secs(2));

        assert_eq!(message, formatdoc! {"
            Failed to start services: the service manager did not respond within 2.0s.
            Its log ends with:
              ERR error=\"bind: no such file or directory\"
            The full log is at {current}.",
            current = current.display(),
        });
    }
}
