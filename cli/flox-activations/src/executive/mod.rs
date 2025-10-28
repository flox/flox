use std::collections::HashMap;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, anyhow};
use flox_core::activate_data::ActivateData;
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

            // sleep indefinitely to simulate running executive
            // In a real scenario, this would be the main loop of the executive process
            loop {
                std::thread::sleep(std::time::Duration::from_secs(60));
            }
        },
    }
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

    command
        .arg("--watchdog")
        .arg(data.watchdog.to_string_lossy().to_string());

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
