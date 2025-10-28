use std::collections::HashMap;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use flox_core::activate_data::ActivateData;
use flox_core::activations;
use flox_core::proc_status::pid_is_running;
use flox_core::util::default_nix_env_vars;
use log::debug;
use nix::sys::wait::waitpid;
use nix::unistd::{close, fork, ForkResult, Pid};

use crate::cli::activate::{
    FLOX_ACTIVATE_START_SERVICES_VAR, FLOX_ACTIVE_ENVIRONMENTS_VAR, FLOX_ENV_LOG_DIR_VAR,
    FLOX_PROMPT_ENVIRONMENTS_VAR, FLOX_RUNTIME_DIR_VAR, FLOX_SERVICES_SOCKET_VAR,
    FLOX_SERVICES_TO_START_VAR,
};

/// The Executive process manages the lifecycle of an activation.
/// It is responsible for:
/// - Forking and execing the activate script
/// - Waiting for the activation to complete
/// - Daemonizing (closing stdio)
/// - Signaling the parent that activation is ready
/// - Monitoring the parent process and cleaning up when it dies
pub fn executive(
    data: ActivateData,
    parent_pid: Pid,
    activation_state_dir: PathBuf,
    activation_id: String,
) -> Result<()> {
    debug!("executive called with parent_pid: {}", parent_pid);

    // Fork a second time to create the activation process
    match unsafe { fork() }.context("Failed to fork activation process")? {
        ForkResult::Child => {
            // Child: exec the activate script
            debug!("Executive child: executing activation script");
            exec_activate_script(data, activation_state_dir, activation_id)?;
            unreachable!("exec should never return");
        },
        ForkResult::Parent { child } => {
            // Parent: wait for activation to complete, then daemonize and signal parent
            debug!("Executive parent: waiting for activation child {}", child);

            // Wait for the activation child to complete
            match waitpid(child, None) {
                Ok(status) => {
                    debug!("Activation child {} exited with status: {:?}", child, status);
                },
                Err(e) => {
                    return Err(anyhow!("Failed to wait for activation child: {}", e));
                },
            }

            // Signal the parent that activation is ready
            debug!("Sending SIGUSR1 to parent {}", parent_pid);
            unsafe {
                libc::kill(parent_pid.as_raw(), libc::SIGUSR1);
            }

            // Daemonize by closing stdin, stdout, stderr
            debug!("Daemonizing: closing stdio");
            close(0).context("Failed to close stdin")?;
            close(1).context("Failed to close stdout")?;
            close(2).context("Failed to close stderr")?;

            // Main monitoring loop: await death of parent PID and all registry PIDs
            monitoring_loop(parent_pid, &data, &activation_state_dir, &activation_id)?;

            // If we reach here, all PIDs are dead - proceed with cleanup
            Ok(())
        },
    }
}

/// Main monitoring loop for the Executive process.
///
/// Monitors both the parent PID and registry PIDs, and exits when both are dead.
/// This implements the flow from the refactor diagram:
/// - n94: Initialize metrics (placeholder)
/// - n100: Submit spooled metrics (placeholder)
/// - ns: Await death of ppid() AND registry_PIDs
/// - n66: stop_process-compose() (placeholder)
/// - ny: Clean up state
/// - nv: Rust Destructors (handled automatically on function exit)
fn monitoring_loop(
    parent_pid: Pid,
    data: &ActivateData,
    activation_state_dir: &Path,
    activation_id: &str,
) -> Result<()> {
    // n94: Initialize metrics, etc. (placeholder)
    debug!("Initializing executive monitoring loop");

    // n100: Submit spooled metrics (placeholder)
    debug!("Submitting spooled metrics (placeholder)");

    // n ns: Main monitoring loop - await death of ppid AND registry PIDs
    let activations_json_path = activations::activations_json_path(&data.flox_runtime_dir, &data.env);
    let poll_interval = Duration::from_secs(1);

    loop {
        // Check if parent PID is still alive
        let parent_alive = pid_is_running(parent_pid.as_raw());

        // Check if there are any PIDs attached to our activation in the registry
        let registry_pids_exist = check_registry_pids(&activations_json_path, activation_id)?;

        if !parent_alive && !registry_pids_exist {
            debug!(
                "Parent PID {} is dead and no registry PIDs remain for activation {}",
                parent_pid, activation_id
            );
            break;
        }

        if !parent_alive {
            debug!(
                "Parent PID {} is dead, but registry PIDs still exist for activation {}",
                parent_pid, activation_id
            );
        }

        // Sleep before next poll
        thread::sleep(poll_interval);
    }

    debug!("Monitoring loop complete, proceeding with cleanup");

    // n66: stop_process-compose() (placeholder)
    debug!("Stopping process-compose (placeholder)");

    // ny: Clean up state (remove temp files, etc.)
    cleanup_activation_state(activation_state_dir)?;

    // nv: Rust Destructors will submit metrics, etc. (handled automatically)
    debug!("Executive monitoring loop exiting");

    Ok(())
}

/// Check if there are any PIDs attached to our activation in the registry.
/// This reads activations.json and checks if our activation has any living PIDs.
fn check_registry_pids(activations_json_path: &Path, activation_id: &str) -> Result<bool> {
    // Read the activations file
    let (activations, _lock) = activations::read_activations_json(activations_json_path)?;

    let Some(activations) = activations else {
        // No activations file means no PIDs
        return Ok(false);
    };

    let activations = activations.check_version().map_err(|e| {
        anyhow!("Failed to check activations version: {}", e)
    })?;

    // Find our activation
    let Some(activation) = activations.activation_for_id_ref(activation_id) else {
        // Our activation is gone, so no PIDs
        return Ok(false);
    };

    // Check if there are any attached PIDs
    // The activation should have already been pruned by the registry logic,
    // so if there are PIDs here, they should be alive
    Ok(!activation.attached_pids().is_empty())
}

/// Clean up the activation state directory and any temporary files.
fn cleanup_activation_state(activation_state_dir: &Path) -> Result<()> {
    debug!("Cleaning up activation state: {:?}", activation_state_dir);

    // Remove the activation state directory
    if activation_state_dir.exists() {
        std::fs::remove_dir_all(activation_state_dir)
            .context("Failed to remove activation state directory")?;
        debug!("Removed activation state directory: {:?}", activation_state_dir);
    }

    Ok(())
}

/// Exec the activate script (bash .../activate)
/// This function never returns on success
fn exec_activate_script(
    data: ActivateData,
    activation_state_dir: PathBuf,
    activation_id: String,
) -> Result<()> {
    let mut exports = HashMap::from([
        (FLOX_ACTIVE_ENVIRONMENTS_VAR, data.flox_active_environments),
        (FLOX_ENV_LOG_DIR_VAR, data.flox_env_log_dir),
        ("FLOX_PROMPT_COLOR_1", data.prompt_color_1),
        ("FLOX_PROMPT_COLOR_2", data.prompt_color_2),
        (FLOX_PROMPT_ENVIRONMENTS_VAR, data.flox_prompt_environments),
        ("_FLOX_SET_PROMPT", data.set_prompt.to_string()),
        (
            "_FLOX_ACTIVATE_STORE_PATH",
            data.flox_activate_store_path.clone(),
        ),
        (FLOX_RUNTIME_DIR_VAR, data.flox_runtime_dir.clone()),
        ("_FLOX_ENV_CUDA_DETECTION", data.flox_env_cuda_detection),
        (
            FLOX_ACTIVATE_START_SERVICES_VAR,
            data.flox_activate_start_services.to_string(),
        ),
        (FLOX_SERVICES_SOCKET_VAR, data.flox_services_socket),
    ]);

    if let Some(services_to_start) = data.flox_services_to_start {
        exports.insert(FLOX_SERVICES_TO_START_VAR, services_to_start);
    }

    exports.extend(default_nix_env_vars());

    let activate_path = data.interpreter_path.join("activate");
    let mut command = Command::new(activate_path);
    command.envs(exports);

    command.arg("--env").arg(&data.env);
    command
        .arg("--env-project")
        .arg(data.env_project.to_string_lossy().to_string());
    command
        .arg("--env-cache")
        .arg(data.env_cache.to_string_lossy().to_string());
    command.arg("--env-description").arg(data.env_description);

    command.arg("--shell").arg(data.shell.exe_path());

    // Add activation-specific arguments
    command.arg("--mode").arg("start");
    command
        .arg("--activation-state-dir")
        .arg(activation_state_dir.to_string_lossy().to_string());
    command.arg("--activation-id").arg(&activation_id);

    debug!("Execing activate script: {:?}", command);

    // Hooks may use stdin, stdout, stderr, so inherit them
    command
        .stderr(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stdin(Stdio::inherit());

    // exec replaces the current process - should never return
    Err(command.exec().into())
}
