use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use flox_core::activate_data::ActivateData;
use flox_core::activations;
use flox_core::proc_status::pid_is_running;
use log::debug;
use nix::sys::wait::waitpid;
use nix::unistd::{ForkResult, Pid, close, fork};

use crate::cli::activate::build_activation_env_vars;

/// A writer for the Executive log file that writes explicitly and immediately to disk.
/// This ensures logging works regardless of the logging subsystem configuration.
struct LogWriter {
    file: Arc<Mutex<File>>,
}

impl LogWriter {
    /// Opens the executive log file and writes the startup message.
    fn new(log_dir: &str, activation_id: &str, project_dir: &str) -> Result<Self> {
        // Create the log directory if it doesn't exist
        std::fs::create_dir_all(log_dir).context("Failed to create log directory")?;

        // Create the log file path
        let log_file_path = PathBuf::from(log_dir).join(format!("executive-{}.log", activation_id));

        // Open the log file for appending (create if it doesn't exist)
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_file_path)
            .context("Failed to open executive log file")?;

        // Get current PID
        let pid = std::process::id();

        // Write startup message immediately with pid prefix for consistency
        let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let msg = format!(
            "[{}] pid={} starting executive project_dir {} activation_id {}\n",
            timestamp, pid, project_dir, activation_id
        );
        file.write_all(msg.as_bytes())
            .context("Failed to write startup message to log")?;
        file.flush().context("Failed to flush log file")?;

        Ok(LogWriter {
            file: Arc::new(Mutex::new(file)),
        })
    }

    /// Writes a log message to the file with timestamp and PID.
    /// The PID is included on every line to help diagnose issues where multiple
    /// executives might write to the same log file.
    fn log(&self, msg: &str) {
        if let Ok(mut file) = self.file.lock() {
            let pid = std::process::id();
            let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
            let formatted = format!("[{}] pid={} {}\n", timestamp, pid, msg);
            let _ = file.write_all(formatted.as_bytes());
            let _ = file.flush();
        }
    }

    /// Writes the shutdown message to the log.
    fn shutdown(&self, activation_id: &str) {
        let msg = format!("shutting down executive activation_id {}", activation_id);
        self.log(&msg);
    }
}

impl Clone for LogWriter {
    fn clone(&self) -> Self {
        LogWriter {
            file: Arc::clone(&self.file),
        }
    }
}

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
            // Parent: This is the Executive process
            // Create the log writer immediately to ensure all logs are captured
            let log = LogWriter::new(&data.flox_env_log_dir, &activation_id, &data.env)
                .context("Failed to create executive log writer")?;

            log.log(&format!("executive parent: waiting for activation child {}", child));

            // Wait for the activation child to complete
            match waitpid(child, None) {
                Ok(status) => {
                    log.log(&format!("activation child {} exited with status: {:?}", child, status));
                },
                Err(e) => {
                    log.log(&format!("failed to wait for activation child: {}", e));
                    return Err(anyhow!("Failed to wait for activation child: {}", e));
                },
            }

            // Replay the environment from the activation script
            // This ensures process-compose inherits the correct environment
            log.log("replaying environment from activation");
            if let Err(e) = crate::shell_gen::capture::replay_env(
                activation_state_dir.join("start.env.json"),
                activation_state_dir.join("end.env.json"),
            ) {
                log.log(&format!("failed to replay environment: {}", e));
                // Continue anyway - this is not fatal
            }

            // n148: Start process-compose daemon (only if service-config.yaml exists)
            // This must happen BEFORE closing stdio so process-compose can start properly
            let service_config_path = PathBuf::from(&data.env).join("service-config.yaml");
            let socket_path = PathBuf::from(&data.flox_services_socket);
            let process_compose_started = if service_config_path.exists() {
                log.log(&format!("starting process-compose daemon with config: {:?}", service_config_path));

                // Only pass services to start if flox_activate_start_services is true
                let services_to_start: Option<Vec<String>> = if data.flox_activate_start_services {
                    data.flox_services_to_start.as_ref().and_then(|json| {
                        serde_json::from_str(json)
                            .inspect_err(|e| log.log(&format!("failed to parse services JSON: {}", e)))
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
                    log.log(&format!("failed to start process-compose: {}", e));
                    // Continue anyway - services failure shouldn't break activation
                    false
                } else {
                    true
                }
            } else {
                log.log("no service-config.yaml found, skipping process-compose startup");
                false
            };

            // n136: Daemonize by closing stdin, stdout, stderr
            log.log("daemonizing: closing stdin/stdout/stderr");
            close(0).context("Failed to close stdin")?;
            close(1).context("Failed to close stdout")?;
            close(2).context("Failed to close stderr")?;

            // Set process title to show "executive: <original command>" in ps listings
            let process_title = format!("executive: {}", data.original_argv.join(" "));
            if let Err(e) = crate::proctitle::setproctitle(&process_title) {
                log.log(&format!("failed to set process title: {}", e));
                // Continue execution even if this fails
            }

            // n130: Signal the parent that activation is ready
            log.log(&format!("sending SIGUSR1 to parent {}", parent_pid));
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
                &log,
            )?;

            // If we reach here, all PIDs are dead - write shutdown message
            log.shutdown(&activation_id);

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
    process_compose_started: bool,
    log: &LogWriter,
) -> Result<()> {
    // n94: Initialize metrics, etc. (placeholder)
    log.log(&format!("initializing monitoring loop for activation {}", activation_id));

    // n100: Submit spooled metrics (placeholder)
    log.log("submitting spooled metrics (placeholder)");

    // n ns: Main monitoring loop - await death of ppid AND registry PIDs
    let activations_json_path =
        activations::activations_json_path(&data.flox_runtime_dir, &data.env);
    let poll_interval = Duration::from_secs(1);

    log.log(&format!("starting monitoring loop - parent_pid={}, activation_id={}", parent_pid, activation_id));

    loop {
        // Check if parent PID is still alive
        let parent_alive = pid_is_running(parent_pid.as_raw());

        // Check if there are any PIDs attached to our activation in the registry
        let registry_pids_exist = check_registry_pids(
            &activations_json_path,
            activation_id,
            &data.flox_runtime_dir,
            &data.env,
            log,
        )?;

        if !parent_alive && !registry_pids_exist {
            log.log(&format!(
                "parent PID {} is dead and no registry PIDs remain for activation {}",
                parent_pid, activation_id
            ));
            break;
        }

        if !parent_alive {
            log.log(&format!(
                "parent PID {} is dead, but registry PIDs still exist for activation {}",
                parent_pid, activation_id
            ));
        }

        // Sleep before next poll
        thread::sleep(poll_interval);
    }

    log.log("monitoring loop complete, proceeding with cleanup");

    // n66: stop_process-compose() (only if we started it)
    if process_compose_started {
        log.log("stopping process-compose");
        let socket_path = PathBuf::from(&data.flox_services_socket);
        if let Err(e) = crate::process_compose::stop_process_compose(&socket_path) {
            log.log(&format!("failed to stop process-compose: {}", e));
            // Continue with cleanup anyway
        }
    }

    // Remove the activation entry from activations.json
    log.log(&format!("removing activation {} from registry", activation_id));
    let (activations, lock) = activations::read_activations_json(&activations_json_path)?;
    if let Some(activations) = activations {
        if let Ok(mut activations) = activations.check_version() {
            activations.remove_activation(activation_id);
            let is_empty = activations.is_empty();

            if is_empty {
                // Last activation removed - clean up the entire registry directory
                log.log("last activation removed, cleaning up registry directory");

                // Get the parent directory (contains activations.json and activations.lock)
                let registry_dir = activations_json_path.parent().expect("activations.json has parent");

                // Rename directory to make it unique before removal
                let pid = std::process::id();
                let remove_dir = registry_dir.with_extension(format!("remove.{}", pid));

                if let Err(e) = std::fs::rename(registry_dir, &remove_dir) {
                    log.log(&format!("failed to rename registry directory for removal: {}", e));
                    // Continue with cleanup anyway
                } else {
                    // Explicitly remove the files
                    let json_path = remove_dir.join("activations.json");
                    let lock_path = remove_dir.join("activations.lock");

                    if let Err(e) = std::fs::remove_file(&json_path) {
                        log.log(&format!("failed to remove activations.json: {}", e));
                    }

                    if let Err(e) = std::fs::remove_file(&lock_path) {
                        log.log(&format!("failed to remove activations.lock: {}", e));
                    }

                    // Remove the directory itself (non-recursively)
                    if let Err(e) = std::fs::remove_dir(&remove_dir) {
                        log.log(&format!("failed to remove registry directory: {}", e));
                    } else {
                        log.log("successfully removed registry directory");
                    }
                }
            } else {
                // Still have activations, just write back the updated registry
                if let Err(e) = activations::write_activations_json(&activations, &activations_json_path, lock) {
                    log.log(&format!("failed to remove activation from registry: {}", e));
                    // Continue with cleanup anyway
                }
            }
        } else {
            log.log("invalid version in activations.json, skipping registry cleanup");
        }
    }

    // ny: Clean up state (remove temp files, etc.)
    cleanup_activation_state(activation_state_dir, log)?;

    Ok(())
}

/// Check if there are any PIDs attached to our activation in the registry.
/// This reads activations.json, prunes dead PIDs, and checks if any living PIDs remain.
///
/// IMPORTANT: This function prunes dead PIDs before checking, ensuring that the
/// executive only waits for actually living processes.
///
/// If the last PID is removed, this also cleans up the activation state directory.
fn check_registry_pids(
    activations_json_path: &Path,
    activation_id: &str,
    runtime_dir: impl AsRef<Path>,
    flox_env: impl AsRef<Path>,
    log: &LogWriter,
) -> Result<bool> {
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
        log.log(&format!("pruned dead PIDs from activation {}", activation_id));
    }

    // Check if there are any PIDs remaining after pruning
    let pids_remain = !activation.attached_pids().is_empty();

    // If no PIDs remain after pruning, remove the activation and clean up its state directory
    if pids_removed && !pids_remain {
        log.log(&format!("last PID removed from activation {}, cleaning up activation state", activation_id));

        // Remove the activation from the registry
        activations.remove_activation(activation_id);

        // Write back the updated activations
        activations::write_activations_json(&activations, activations_json_path, lock)?;
        log.log(&format!("removed activation {} from registry", activation_id));

        // Clean up the activation state directory
        let activation_state_dir =
            activations::activation_state_dir_path(runtime_dir, flox_env, activation_id)?;

        // Remove the specific files we know about
        let add_env_path = activation_state_dir.join("add.env");
        let del_env_path = activation_state_dir.join("del.env");
        let start_json_path = activation_state_dir.join("start.env.json");
        let end_json_path = activation_state_dir.join("end.env.json");

        // Remove files if they exist (ignore errors)
        if add_env_path.exists() {
            if let Err(e) = std::fs::remove_file(&add_env_path) {
                log.log(&format!("failed to remove add.env: {}", e));
            }
        }
        if del_env_path.exists() {
            if let Err(e) = std::fs::remove_file(&del_env_path) {
                log.log(&format!("failed to remove del.env: {}", e));
            }
        }
        if start_json_path.exists() {
            if let Err(e) = std::fs::remove_file(&start_json_path) {
                log.log(&format!("failed to remove start.env.json: {}", e));
            }
        }
        if end_json_path.exists() {
            if let Err(e) = std::fs::remove_file(&end_json_path) {
                log.log(&format!("failed to remove end.env.json: {}", e));
            }
        }

        // Remove the directory itself (non-recursively)
        if let Err(e) = std::fs::remove_dir(&activation_state_dir) {
            log.log(&format!("failed to remove activation state directory: {} (may not be empty or may not exist)", e));
        } else {
            log.log("successfully removed activation state directory");
        }
    } else if pids_removed {
        // PIDs were removed but some remain, just write back the updated registry
        activations::write_activations_json(&activations, activations_json_path, lock)?;
        log.log("updated activations.json after pruning dead PIDs");
    }

    Ok(pids_remain)
}

/// Clean up the activation state directory and any temporary files.
fn cleanup_activation_state(activation_state_dir: &Path, log: &LogWriter) -> Result<()> {
    log.log(&format!("cleaning up activation state: {:?}", activation_state_dir));

    // Remove the activation state directory
    if activation_state_dir.exists() {
        std::fs::remove_dir_all(activation_state_dir)
            .context("Failed to remove activation state directory")?;
        log.log(&format!("removed activation state directory: {:?}", activation_state_dir));
    } else {
        log.log(&format!("activation state directory already removed: {:?}", activation_state_dir));
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

    command.arg("--shell").arg(data.shell.exe_path());

    // Add activation-specific arguments
    command.arg("--mode").arg(data.mode);
    command
        .arg("--activation-state-dir")
        .arg(activation_state_dir.to_string_lossy().to_string());

    debug!("Execing activate script: {:?}", command);

    // Hooks may use stdin, stdout, stderr, so inherit them
    command
        .stderr(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stdin(Stdio::inherit());

    // exec replaces the current process - should never return
    Err(command.exec().into())
}
