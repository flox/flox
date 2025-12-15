use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

use flox_core::activate::context::{ActivateCtx, InvocationType};
use flox_core::activate::vars::{FLOX_ACTIVE_ENVIRONMENTS_VAR, FLOX_RUNTIME_DIR_VAR};
use flox_core::util::default_nix_env_vars;
use is_executable::IsExecutable;

use crate::cli::activate::VarsFromEnvironment;
use crate::cli::fix_paths::{fix_manpath_var, fix_path_var};
use crate::cli::set_env_dirs::fix_env_dirs_var;
use crate::cli::start_or_attach::StartOrAttachResult;
use crate::env_diff::EnvDiff;
pub const FLOX_ENV_LOG_DIR_VAR: &str = "_FLOX_ENV_LOG_DIR";
pub const FLOX_PROMPT_ENVIRONMENTS_VAR: &str = "FLOX_PROMPT_ENVIRONMENTS";
/// This variable is used to communicate what socket to use to the activate
/// script.
pub const FLOX_SERVICES_SOCKET_VAR: &str = "_FLOX_SERVICES_SOCKET";

pub const FLOX_ACTIVATE_START_SERVICES_VAR: &str = "FLOX_ACTIVATE_START_SERVICES";
pub const FLOX_ENV_DIRS_VAR: &str = "FLOX_ENV_DIRS";

pub(super) fn assemble_command_for_start_script(
    context: ActivateCtx,
    subsystem_verbosity: u32,
    vars_from_env: VarsFromEnvironment,
    start_or_attach_result: &StartOrAttachResult,
    invocation_type: InvocationType,
) -> Command {
    let mut command = Command::new(context.interpreter_path.join("activate.d/start.bash"));
    add_old_cli_options(&mut command, &context);
    command.envs(old_cli_envs(context.clone()));
    add_old_activate_script_exports(
        &mut command,
        &context,
        subsystem_verbosity,
        vars_from_env,
        start_or_attach_result,
    );
    add_start_script_options(&mut command, start_or_attach_result, invocation_type);
    command
}

/// Set (and unset) environment variables needed to be activated
pub fn apply_activation_env(
    command: &mut Command,
    context: ActivateCtx,
    subsystem_verbosity: u32,
    vars_from_env: VarsFromEnvironment,
    env_diff: &EnvDiff,
    start_or_attach_result: &StartOrAttachResult,
) {
    command.envs(old_cli_envs(context.clone()));
    add_old_activate_script_exports(
        command,
        &context,
        subsystem_verbosity,
        vars_from_env,
        start_or_attach_result,
    );
    command.envs(&env_diff.additions);
    for var in &env_diff.deletions {
        command.env_remove(var);
    }
}

pub fn old_cli_envs(context: ActivateCtx) -> HashMap<&'static str, String> {
    let mut exports = HashMap::from([
        (
            FLOX_ACTIVE_ENVIRONMENTS_VAR,
            context.flox_active_environments,
        ),
        ("FLOX_PROMPT_COLOR_1", context.prompt_color_1),
        ("FLOX_PROMPT_COLOR_2", context.prompt_color_2),
        // Set `FLOX_PROMPT_ENVIRONMENTS` to the constructed prompt string,
        // which may be ""
        // This is used by set-prompt script, and tcsh in particular does not
        // tolerate references to undefined variables.
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
        // This is user-facing and documented
        (
            FLOX_ACTIVATE_START_SERVICES_VAR,
            context.flox_activate_start_services.to_string(),
        ),
    ]);
    if let Some(log_dir) = context.flox_env_log_dir.as_ref() {
        exports.insert(
            FLOX_ENV_LOG_DIR_VAR,
            log_dir.clone().to_string_lossy().to_string(),
        );
    }
    if let Some(socket_path) = context.flox_services_socket.as_ref() {
        exports.insert(
            FLOX_SERVICES_SOCKET_VAR,
            socket_path.clone().to_string_lossy().to_string(),
        );
    }

    exports.extend(default_nix_env_vars());

    exports
}

/// Prior to the refactor, these options were passed by the CLI to the activate
/// script
fn add_old_cli_options(command: &mut Command, context: &ActivateCtx) {
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
        .arg(context.env_description.clone());

    // Pass down the activation mode
    command.arg("--mode").arg(context.mode.to_string());

    command.arg("--shell").arg(context.shell.exe_path());
}

/// Options parsed by getopt that are only used by start.bash
fn add_start_script_options(
    command: &mut Command,
    start_or_attach_result: &StartOrAttachResult,
    invocation_type: InvocationType,
) {
    command.args([
        "--activation-state-dir",
        &start_or_attach_result
            .activation_state_dir
            .to_string_lossy(),
        "--invocation-type",
        &invocation_type.to_string(),
        "--activation-id",
        &start_or_attach_result.activation_id,
    ]);
}

/// Prior to the refactor, these variables were exported in the activate script
// TODO: we still use std::env::var in this function,
// so we should either drop those uses and get those vars in VarsFromEnvironment,
// or we should completely drop VarsFromEnvironment .
fn add_old_activate_script_exports(
    command: &mut Command,
    context: &ActivateCtx,
    subsystem_verbosity: u32,
    vars_from_environment: VarsFromEnvironment,
    start_or_attach_result: &StartOrAttachResult,
) {
    let mut removals = Vec::new();
    let mut exports = HashMap::from([
        ("_flox_activate_tracelevel", subsystem_verbosity.to_string()),
        // Propagate required variables that are documented as exposed.
        ("FLOX_ENV", context.env.clone()),
        (
            "FLOX_ENV_CACHE",
            context.env_cache.to_string_lossy().to_string(),
        ),
        ("FLOX_ENV_DESCRIPTION", context.env_description.clone()),
        (
            "_FLOX_DOT_FLOX_PATH",
            context.dot_flox_path.to_string_lossy().to_string(),
        ),
        (
            "_FLOX_ACTIVATION_STATE_DIR",
            start_or_attach_result
                .activation_state_dir
                .to_string_lossy()
                .to_string(),
        ),
        (
            "_FLOX_ACTIVATION_ID",
            start_or_attach_result.activation_id.clone(),
        ),
        // These are used by various scripts...custom ZDOTDIR files, set-prompt,
        // .tcshrc
        (
            "_flox_activate_tracer",
            activate_tracer(&context.interpreter_path),
        ),
        (
            "_activate_d",
            context
                .interpreter_path
                .join("activate.d")
                .to_string_lossy()
                .to_string(),
        ),
    ]);
    // Propagate optional variables that are documented as exposed.
    // NB: `generate_*_start_commands()` performs the same logic except for zsh.
    if let Some(env_project) = context.env_project.as_ref() {
        exports.insert(
            "FLOX_ENV_PROJECT",
            env_project.to_string_lossy().to_string(),
        );
    } else {
        removals.push("FLOX_ENV_PROJECT");
    }

    exports.extend(fixed_vars_to_export(&context.env, vars_from_environment));

    command.envs(&exports);
    for var in &removals {
        command.env_remove(var);
    }
}

/// Calculate values for FLOX_ENV_DIRS, PATH, and MANPATH
fn fixed_vars_to_export(
    flox_env: impl AsRef<str>,
    vars_from_environment: VarsFromEnvironment,
) -> HashMap<&'static str, String> {
    let new_flox_env_dirs = fix_env_dirs_var(
        flox_env.as_ref(),
        vars_from_environment
            .flox_env_dirs
            .unwrap_or("".to_string()),
    );
    let new_path = fix_path_var(&new_flox_env_dirs, &vars_from_environment.path);
    let new_manpath = fix_manpath_var(
        &new_flox_env_dirs,
        &vars_from_environment.manpath.unwrap_or("".to_string()),
    );
    HashMap::from([
        (FLOX_ENV_DIRS_VAR, new_flox_env_dirs),
        ("PATH", new_path),
        ("MANPATH", new_manpath),
    ])
}

/// The activate_tracer is set from the FLOX_ACTIVATE_TRACE env var.
/// If that env var is empty then activate_tracer is set to the full path of the `true` command in the PATH.
/// If that env var is not empty and refers to an executable then then activate_tracer is set to that value.
/// Else activate_tracer is set to refer to {interpreter_path}/activate.d/trace.
// TODO: we should probably pass this around rather than recomputing it
pub fn activate_tracer(interpreter_path: impl AsRef<Path>) -> String {
    if let Ok(trace_path) = std::env::var("FLOX_ACTIVATE_TRACE") {
        if Path::new(&trace_path).is_executable() {
            trace_path
        } else {
            interpreter_path
                .as_ref()
                .join("activate.d")
                .join("trace")
                .to_string_lossy()
                .to_string()
        }
    } else {
        "true".to_string()
    }
}
