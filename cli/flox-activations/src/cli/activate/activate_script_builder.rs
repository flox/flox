use std::collections::HashMap;
use std::process::Command;

use flox_core::activate::context::ActivateCtx;
use flox_core::activate::vars::{FLOX_ACTIVE_ENVIRONMENTS_VAR, FLOX_RUNTIME_DIR_VAR};
use flox_core::util::default_nix_env_vars;
pub const FLOX_ENV_LOG_DIR_VAR: &str = "_FLOX_ENV_LOG_DIR";
pub const FLOX_PROMPT_ENVIRONMENTS_VAR: &str = "FLOX_PROMPT_ENVIRONMENTS";
/// This variable is used to communicate what socket to use to the activate
/// script.
pub const FLOX_SERVICES_SOCKET_VAR: &str = "_FLOX_SERVICES_SOCKET";

pub const FLOX_SERVICES_TO_START_VAR: &str = "_FLOX_SERVICES_TO_START";
pub const FLOX_ACTIVATE_START_SERVICES_VAR: &str = "FLOX_ACTIVATE_START_SERVICES";

pub(super) fn assemble_command_for_activate_script(data: ActivateCtx) -> Command {
    let activate_path = data.interpreter_path.join("activate_temporary");
    let mut command = Command::new(activate_path);
    add_old_cli_options(&mut command, data);
    command
}

fn add_old_cli_options(command: &mut Command, data: ActivateCtx) {
    let mut exports = HashMap::from([
        (FLOX_ACTIVE_ENVIRONMENTS_VAR, data.flox_active_environments),
        ("FLOX_PROMPT_COLOR_1", data.prompt_color_1),
        ("FLOX_PROMPT_COLOR_2", data.prompt_color_2),
        // Set `FLOX_PROMPT_ENVIRONMENTS` to the constructed prompt string,
        // which may be ""
        (FLOX_PROMPT_ENVIRONMENTS_VAR, data.flox_prompt_environments),
        ("_FLOX_SET_PROMPT", data.set_prompt.to_string()),
        ("_FLOX_ACTIVATE_STORE_PATH", data.flox_activate_store_path),
        (
            // TODO: we should probably figure out a more consistent way to
            // pass this since it's also passed for `flox build`
            FLOX_RUNTIME_DIR_VAR,
            data.flox_runtime_dir,
        ),
        ("_FLOX_ENV_CUDA_DETECTION", data.flox_env_cuda_detection),
        (
            FLOX_ACTIVATE_START_SERVICES_VAR,
            data.flox_activate_start_services.to_string(),
        ),
    ]);
    if let Some(log_dir) = data.flox_env_log_dir.as_ref() {
        exports.insert(FLOX_ENV_LOG_DIR_VAR, log_dir.clone());
    }
    if let Some(socket_path) = data.flox_services_socket.as_ref() {
        exports.insert(FLOX_SERVICES_SOCKET_VAR, socket_path.clone());
    }
    if let Some(services_to_start) = data.flox_services_to_start {
        exports.insert(FLOX_SERVICES_TO_START_VAR, services_to_start);
    }

    exports.extend(default_nix_env_vars());

    command.envs(exports);

    command.arg("--env").arg(&data.env);
    if let Some(env_project) = data.env_project.as_ref() {
        command
            .arg("--env-project")
            .arg(env_project.to_string_lossy().to_string());
    }
    command
        .arg("--env-cache")
        .arg(data.env_cache.to_string_lossy().to_string());
    command.arg("--env-description").arg(data.env_description);

    // Pass down the activation mode
    command.arg("--mode").arg(data.mode);

    if let Some(watchdog_bin) = data.watchdog_bin.as_ref() {
        command
            .arg("--watchdog")
            .arg(watchdog_bin.to_string_lossy().to_string());
    }

    command.arg("--shell").arg(data.shell.exe_path());
}
