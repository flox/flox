use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

use flox_core::activate::context::ActivateCtx;
use flox_core::activate::vars::{FLOX_ACTIVE_ENVIRONMENTS_VAR, FLOX_RUNTIME_DIR_VAR};
use flox_core::util::default_nix_env_vars;
use is_executable::IsExecutable;
pub const FLOX_ENV_LOG_DIR_VAR: &str = "_FLOX_ENV_LOG_DIR";
pub const FLOX_PROMPT_ENVIRONMENTS_VAR: &str = "FLOX_PROMPT_ENVIRONMENTS";
/// This variable is used to communicate what socket to use to the activate
/// script.
pub const FLOX_SERVICES_SOCKET_VAR: &str = "_FLOX_SERVICES_SOCKET";

pub const FLOX_SERVICES_TO_START_VAR: &str = "_FLOX_SERVICES_TO_START";
pub const FLOX_ACTIVATE_START_SERVICES_VAR: &str = "FLOX_ACTIVATE_START_SERVICES";

pub(super) fn assemble_command_for_activate_script(
    context: ActivateCtx,
    subsystem_verbosity: u32,
) -> Command {
    let activate_path = context.interpreter_path.join("activate_temporary");
    let mut command = Command::new(activate_path);
    add_old_cli_options(&mut command, context.clone());
    add_old_activate_script_exports(&mut command, &context, subsystem_verbosity);
    command
}

/// Prior to the refactor, these options were passed by the CLI to the activate
/// script
fn add_old_cli_options(command: &mut Command, context: ActivateCtx) {
    let mut exports = HashMap::from([
        (
            FLOX_ACTIVE_ENVIRONMENTS_VAR,
            context.flox_active_environments,
        ),
        ("FLOX_PROMPT_COLOR_1", context.prompt_color_1),
        ("FLOX_PROMPT_COLOR_2", context.prompt_color_2),
        // Set `FLOX_PROMPT_ENVIRONMENTS` to the constructed prompt string,
        // which may be ""
        (
            FLOX_PROMPT_ENVIRONMENTS_VAR,
            context.flox_prompt_environments,
        ),
        ("_FLOX_SET_PROMPT", context.set_prompt.to_string()),
        (
            "_FLOX_ACTIVATE_STORE_PATH",
            context.flox_activate_store_path,
        ),
        (
            // TODO: we should probably figure out a more consistent way to
            // pass this since it's also passed for `flox build`
            FLOX_RUNTIME_DIR_VAR,
            context.flox_runtime_dir,
        ),
        ("_FLOX_ENV_CUDA_DETECTION", context.flox_env_cuda_detection),
        (
            FLOX_ACTIVATE_START_SERVICES_VAR,
            context.flox_activate_start_services.to_string(),
        ),
    ]);
    if let Some(log_dir) = context.flox_env_log_dir.as_ref() {
        exports.insert(FLOX_ENV_LOG_DIR_VAR, log_dir.clone());
    }
    if let Some(socket_path) = context.flox_services_socket.as_ref() {
        exports.insert(FLOX_SERVICES_SOCKET_VAR, socket_path.clone());
    }
    if let Some(services_to_start) = context.flox_services_to_start {
        exports.insert(FLOX_SERVICES_TO_START_VAR, services_to_start);
    }

    exports.extend(default_nix_env_vars());

    command.envs(exports);

    command.arg("--env").arg(&context.env);
    if let Some(env_project) = context.env_project.as_ref() {
        command
            .arg("--env-project")
            .arg(env_project.to_string_lossy().to_string());
    }
    command
        .arg("--env-cache")
        .arg(context.env_cache.to_string_lossy().to_string());
    command
        .arg("--env-description")
        .arg(context.env_description);

    // Pass down the activation mode
    command.arg("--mode").arg(context.mode);

    if let Some(watchdog_bin) = context.watchdog_bin.as_ref() {
        command
            .arg("--watchdog")
            .arg(watchdog_bin.to_string_lossy().to_string());
    }

    command.arg("--shell").arg(context.shell.exe_path());
}

/// Prior to the refactor, these variables were exported in the activate script
fn add_old_activate_script_exports(
    command: &mut Command,
    context: &ActivateCtx,
    subsystem_verbosity: u32,
) {
    let mut exports =
        HashMap::from([("_flox_activate_tracelevel", subsystem_verbosity.to_string())]);

    // The activate_tracer is set from the FLOX_ACTIVATE_TRACE env var.
    // If that env var is empty then activate_tracer is set to the full path of the `true` command in the PATH.
    // If that env var is not empty and refers to an executable then then activate_tracer is set to that value.
    // Else activate_tracer is set to refer to {interpreter_path}/activate.d/trace.
    let activate_tracer = if let Ok(trace_path) = std::env::var("FLOX_ACTIVATE_TRACE") {
        if Path::new(&trace_path).is_executable() {
            trace_path
        } else {
            context
                .interpreter_path
                .join("activate.d")
                .join("trace")
                .to_string_lossy()
                .to_string()
        }
    } else {
        "true".to_string()
    };

    exports.insert("_flox_activate_tracer", activate_tracer);

    command.envs(&exports);
}
