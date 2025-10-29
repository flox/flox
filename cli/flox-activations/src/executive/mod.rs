use std::fs::OpenOptions;
use std::os::unix::io::AsRawFd;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use flox_core::activate_data::ActivateData;
use flox_core::activations;
use flox_core::proc_status::pid_is_running;
use log::debug;
use nix::sys::wait::waitpid;
use nix::unistd::{ForkResult, Pid, close, fork};

use crate::cli::activate::{
    FLOX_ACTIVATE_START_SERVICES_VAR,
    FLOX_ACTIVE_ENVIRONMENTS_VAR,
    FLOX_ENV_LOG_DIR_VAR,
    FLOX_PROMPT_ENVIRONMENTS_VAR,
    FLOX_RUNTIME_DIR_VAR,
    FLOX_SERVICES_SOCKET_VAR,
    FLOX_SERVICES_TO_START_VAR,
    build_activation_env_vars,
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
                    debug!(
                        "Activation child {} exited with status: {:?}",
                        child, status
                    );
                },
                Err(e) => {
                    return Err(anyhow!("Failed to wait for activation child: {}", e));
                },
            }

            // Replay the environment from the activation script
            // This ensures process-compose inherits the correct environment
            debug!("Replaying environment from activation");
            if let Err(e) = crate::shell_gen::capture::replay_env(
                activation_state_dir.join("start.env.json"),
                activation_state_dir.join("end.env.json"),
            ) {
                debug!("Failed to replay environment: {}", e);
                // Continue anyway - this is not fatal
            }

            // n148: Start process-compose daemon (only if service-config.yaml exists)
            // This must happen BEFORE closing stdio so process-compose can start properly
            let service_config_path = PathBuf::from(&data.env).join("service-config.yaml");
            let socket_path = PathBuf::from(&data.flox_services_socket);
            let process_compose_started = if service_config_path.exists() {
                debug!(
                    "Starting process-compose daemon with config: {:?}",
                    service_config_path
                );

                // Only pass services to start if flox_activate_start_services is true
                let services_to_start: Option<Vec<String>> = if data.flox_activate_start_services {
                    data.flox_services_to_start.as_ref().and_then(|json| {
                        serde_json::from_str(json)
                            .inspect_err(|e| debug!("Failed to parse services JSON: {}", e))
                            .ok()
                    })
                } else {
                    None
                };

                if let Err(e) = crate::process_compose::start_process_compose(
                    &service_config_path,
                    &socket_path,
                    services_to_start.as_deref(),
                ) {
                    debug!("Failed to start process-compose: {}", e);
                    // Continue anyway - services failure shouldn't break activation
                    false
                } else {
                    true
                }
            } else {
                debug!("No service-config.yaml found, skipping process-compose startup");
                false
            };

            // n136: Daemonize by closing stdin, stdout, and redirecting stderr to log file
            debug!("Daemonizing: closing stdin/stdout and redirecting stderr to log");
            close(0).context("Failed to close stdin")?;
            close(1).context("Failed to close stdout")?;

            // Redirect stderr to a log file so we can continue logging
            redirect_stderr_to_logfile(&data.flox_env_log_dir, &activation_id)?;

            // Set logging level to Debug for the executive, regardless of original verbosity
            // The logger format is inherited from the parent, we just need to change the level
            log::set_max_level(log::LevelFilter::Debug);

            // Set process title to show "executive: <original command>" in ps listings
            let process_title = format!("executive: {}", data.original_argv.join(" "));
            if let Err(e) = crate::proctitle::setproctitle(&process_title) {
                debug!("Failed to set process title: {}", e);
                // Continue execution even if this fails
            }

            // n130: Signal the parent that activation is ready
            debug!("Sending SIGUSR1 to parent {}", parent_pid);
            unsafe {
                libc::kill(parent_pid.as_raw(), libc::SIGUSR1);
            }

            // Main monitoring loop: await death of parent PID and all registry PIDs
            monitoring_loop(
                parent_pid,
                &data,
                &activation_state_dir,
                &activation_id,
                process_compose_started,
            )?;

            // If we reach here, all PIDs are dead - proceed with cleanup
            Ok(())
        },
    }
}

/// Redirects stderr to a log file for the executive process.
/// This allows us to continue logging after daemonization.
fn redirect_stderr_to_logfile(log_dir: &str, activation_id: &str) -> Result<()> {
    // Create the log directory if it doesn't exist
    std::fs::create_dir_all(log_dir).context("Failed to create log directory")?;

    // Create the log file path
    let log_file_path = PathBuf::from(log_dir).join(format!("executive-{}.log", activation_id));

    // Open the log file for appending (create if it doesn't exist)
    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file_path)
        .context("Failed to open executive log file")?;

    // Redirect stderr (fd 2) to the log file using libc::dup2
    let result = unsafe { libc::dup2(log_file.as_raw_fd(), 2) };
    if result == -1 {
        return Err(anyhow!("Failed to redirect stderr to log file"));
    }

    // Log file will be closed when it goes out of scope, but the fd 2 will remain open
    debug!("Executive stderr redirected to: {:?}", log_file_path);

    Ok(())
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
    process_compose_started: bool,
) -> Result<()> {
    // n94: Initialize metrics, etc. (placeholder)
    debug!(
        "Executive: Initializing monitoring loop for activation {}",
        activation_id
    );

    // n100: Submit spooled metrics (placeholder)
    debug!("Executive: Submitting spooled metrics (placeholder)");

    // n ns: Main monitoring loop - await death of ppid AND registry PIDs
    let activations_json_path =
        activations::activations_json_path(&data.flox_runtime_dir, &data.env);
    let poll_interval = Duration::from_secs(1);

    debug!(
        "Executive: Starting monitoring loop - parent_pid={}, activation_id={}",
        parent_pid, activation_id
    );

    loop {
        // Check if parent PID is still alive
        let parent_alive = pid_is_running(parent_pid.as_raw());

        // Check if there are any PIDs attached to our activation in the registry
        let registry_pids_exist = check_registry_pids(&activations_json_path, activation_id)?;

        if !parent_alive && !registry_pids_exist {
            debug!(
                "Executive: Parent PID {} is dead and no registry PIDs remain for activation {}",
                parent_pid, activation_id
            );
            break;
        }

        if !parent_alive {
            debug!(
                "Executive: Parent PID {} is dead, but registry PIDs still exist for activation {}",
                parent_pid, activation_id
            );
        }

        // Sleep before next poll
        thread::sleep(poll_interval);
    }

    debug!("Executive: Monitoring loop complete, proceeding with cleanup");

    // n66: stop_process-compose() (only if we started it)
    if process_compose_started {
        debug!("Executive: Stopping process-compose");
        let socket_path = PathBuf::from(&data.flox_services_socket);
        if let Err(e) = crate::process_compose::stop_process_compose(&socket_path) {
            debug!("Failed to stop process-compose: {}", e);
            // Continue with cleanup anyway
        }
    }

    // ny: Clean up state (remove temp files, etc.)
    cleanup_activation_state(activation_state_dir)?;

    // nv: Rust Destructors will submit metrics, etc. (handled automatically)
    debug!("Executive: Exiting");

    Ok(())
}

/// Check if there are any PIDs attached to our activation in the registry.
/// This reads activations.json, prunes dead PIDs, and checks if any living PIDs remain.
///
/// IMPORTANT: This function prunes dead PIDs before checking, ensuring that the
/// executive only waits for actually living processes.
fn check_registry_pids(activations_json_path: &Path, activation_id: &str) -> Result<bool> {
    // Read the activations file with lock
    let (activations, lock) = activations::read_activations_json(activations_json_path)?;

    let Some(activations) = activations else {
        // No activations file means no PIDs
        return Ok(false);
    };

    let mut activations = activations
        .check_version()
        .map_err(|e| anyhow!("Failed to check activations version: {}", e))?;

    // Find our activation
    let Some(activation) = activations.activation_for_id_mut(activation_id) else {
        // Our activation is gone, so no PIDs
        return Ok(false);
    };

    // Prune any dead PIDs from the registry using kill(0)
    // This is critical - without this, we'll wait forever for dead PIDs
    let pids_removed = activation.remove_terminated_pids();

    if pids_removed {
        debug!(
            "Executive: Pruned dead PIDs from activation {}",
            activation_id
        );
    }

    // Check if there are any PIDs remaining after pruning
    let pids_remain = !activation.attached_pids().is_empty();

    // Write back the pruned activations if we removed any PIDs
    if pids_removed {
        activations::write_activations_json(&activations, activations_json_path, lock)?;
        debug!("Executive: Updated activations.json after pruning dead PIDs");
    }

    Ok(pids_remain)
}

/// Clean up the activation state directory and any temporary files.
fn cleanup_activation_state(activation_state_dir: &Path) -> Result<()> {
    debug!(
        "Executive: Cleaning up activation state: {:?}",
        activation_state_dir
    );

    // Remove the activation state directory
    if activation_state_dir.exists() {
        std::fs::remove_dir_all(activation_state_dir)
            .context("Failed to remove activation state directory")?;
        debug!(
            "Executive: Removed activation state directory: {:?}",
            activation_state_dir
        );
    } else {
        debug!(
            "Executive: Activation state directory already removed: {:?}",
            activation_state_dir
        );
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
    let exports = build_activation_env_vars(&data);

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
