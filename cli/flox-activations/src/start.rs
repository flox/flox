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
use flox_core::activate::context::ActivateCtx;
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
use nix::sys::signal::{Signal, kill};
use nix::sys::wait::{WaitPidFlag, WaitStatus, waitpid};
use nix::unistd::{Pid, getpid};
use signal_hook::consts::{SIGCHLD, SIGUSR1};
use signal_hook::iterator::Signals;
use tracing::{debug, error};

use crate::activate_script_builder::assemble_activate_command;
use crate::cli::executive::ExecutiveCtx;
use crate::process_compose::{
    process_compose_down,
    start_services_via_socket,
    wait_for_socket_ready,
};
use crate::vars_from_env::VarsFromEnvironment;

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
    let start_state_dir = start_id.state_dir_path(
        &context.attach_ctx.flox_runtime_dir,
        &context.attach_ctx.dot_flox_path,
    )?;
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&start_state_dir)?;

    let new_executive = if !activations.executive_started() {
        // Register signal handler BEFORE spawning executive to avoid race condition
        // where SIGUSR1 arrives before handler is registered
        let signals = Signals::new([SIGCHLD, SIGUSR1])?;
        let exec_pid = spawn_executive(context, &start_state_dir)?;
        activations.set_executive_pid(exec_pid.as_raw());
        Some((exec_pid, signals))
    } else {
        None
    };

    write_activations_json(activations, activations_json_path, lock)?;

    if let Some((exec_pid, signals)) = new_executive {
        wait_for_executive(exec_pid, signals)?;
    }

    let mut start_command = assemble_activate_command(
        context.clone(),
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

    // Re-acquire lock to mark ready
    let (activations_opt, lock) = read_activations_json(activations_json_path)?;
    let mut activations = activations_opt.expect("activations.json should exist");
    activations.set_ready(&start_id);
    write_activations_json(&activations, activations_json_path, lock)?;

    Ok(StartOrAttachResult::Start { start_id })
}

/// Start services with a new process-compose instance.
///
/// The CLI has already decided that a new process-compose is needed.
/// This function starts process-compose and then starts the specified services.
pub fn start_services_with_new_process_compose(
    runtime_dir: &str,
    dot_flox_path: &Path,
    process_compose_bin: &Path,
    socket_path: &Path,
    services: &[String],
) -> Result<(), anyhow::Error> {
    let activations_json_path = state_json_path(runtime_dir, dot_flox_path);
    let (activations_opt, lock) = read_activations_json(&activations_json_path)?;
    let activations = activations_opt.expect("state.json should exist");
    let executive_pid = activations.executive_pid();
    // Don't hold a lock because the executive will need it when starting `process-compose`
    drop(lock);

    debug!("starting new process-compose for services");
    signal_new_process_compose(process_compose_bin, socket_path, executive_pid)?;
    start_services_via_socket(process_compose_bin, socket_path, services)?;

    Ok(())
}

/// Start a new process-compose instance by signaling the executive.
fn signal_new_process_compose(
    process_compose_bin: &Path,
    socket_path: &Path,
    executive_pid: i32,
) -> Result<(), anyhow::Error> {
    // Stop first, if running, to ensure that we wait on the socket from the new instance.
    if socket_path.exists() {
        debug!("shutting down old process-compose");
        if let Err(err) = process_compose_down(process_compose_bin, socket_path) {
            error!(%err, "failed to stop process-compose");
        }
    }

    debug!(
        executive_pid,
        "sending SIGUSR1 to executive to start new process-compose",
    );
    kill(Pid::from_raw(executive_pid), Signal::SIGUSR1)?;

    let activation_timeout = std::env::var("_FLOX_SERVICES_ACTIVATE_TIMEOUT")
        .ok()
        .and_then(|t| t.parse().ok())
        .map(Duration::from_secs_f64)
        .unwrap_or(Duration::from_secs(2));
    let socket_ready = wait_for_socket_ready(process_compose_bin, socket_path, activation_timeout)?;
    if !socket_ready {
        // TODO: We used to print the services log (if it exists) here to
        // help users debug the failure but we no longer have the path
        // available now that it's started by the executive.
        bail!("Failed to start services: process-compose socket not ready");
    }

    Ok(())
}

fn spawn_executive(context: &ActivateCtx, start_state_dir: &Path) -> Result<Pid, anyhow::Error> {
    let parent_pid = getpid();

    // Serialize ExecutiveCtx
    let executive_ctx = ExecutiveCtx {
        context: context.clone(),
        parent_pid: parent_pid.as_raw(),
    };

    let temp_file = tempfile::NamedTempFile::with_prefix_in("executive_ctx_", start_state_dir)?;
    serde_json::to_writer(&temp_file, &executive_ctx)?;
    let executive_ctx_path = temp_file.path().to_path_buf();
    temp_file.keep()?;

    // Spawn executive
    let mut executive = Command::new((*FLOX_ACTIVATIONS_BIN).clone());
    executive.args([
        "executive",
        "--dot-flox-path",
        &context.attach_ctx.dot_flox_path.to_string_lossy(),
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
