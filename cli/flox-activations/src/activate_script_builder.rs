use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

use flox_core::activate::context::{ActivateCoreCtx, ActivateCtx, ActivateProjectCtx};
use flox_core::activate::vars::{FLOX_ACTIVE_ENVIRONMENTS_VAR, FLOX_RUNTIME_DIR_VAR};
use flox_core::util::default_nix_env_vars;
use is_executable::IsExecutable;

use crate::cli::fix_paths::{fix_manpath_var, fix_path_var};
use crate::cli::set_env_dirs::fix_env_dirs_var;
use crate::env_diff::EnvDiff;
use crate::vars_from_env::VarsFromEnvironment;
pub const FLOX_ENV_LOG_DIR_VAR: &str = "_FLOX_ENV_LOG_DIR";
pub const FLOX_PROMPT_ENVIRONMENTS_VAR: &str = "FLOX_PROMPT_ENVIRONMENTS";
/// This variable is used to communicate what socket to use to the activate
/// script.
pub const FLOX_SERVICES_SOCKET_VAR: &str = "_FLOX_SERVICES_SOCKET";

pub const FLOX_ACTIVATE_START_SERVICES_VAR: &str = "FLOX_ACTIVATE_START_SERVICES";
pub const FLOX_ENV_DIRS_VAR: &str = "FLOX_ENV_DIRS";

pub(super) fn assemble_activate_command(
    context: ActivateCtx,
    subsystem_verbosity: u32,
    vars_from_env: VarsFromEnvironment,
    state_dir: &Path,
) -> Command {
    let mut command = Command::new(context.core.interpreter_path.join("activate"));
    add_old_cli_options(&mut command, &context);
    command.envs(old_cli_envs(&context.core, Some(&context.project)));
    add_old_activate_script_exports(
        &mut command,
        &context.core,
        Some(&context.project),
        subsystem_verbosity,
        vars_from_env,
        state_dir,
    );
    add_activate_script_options(&mut command, state_dir);
    command
}

/// Set (and unset) environment variables needed to be activated.
/// If project is provided, also includes service-related variables.
pub fn apply_activation_env(
    command: &mut Command,
    core: &ActivateCoreCtx,
    project: Option<&ActivateProjectCtx>,
    subsystem_verbosity: u32,
    vars_from_env: VarsFromEnvironment,
    env_diff: &EnvDiff,
    state_dir: &Path,
) {
    command.envs(old_cli_envs(core, project));
    add_old_activate_script_exports(
        command,
        core,
        project,
        subsystem_verbosity,
        vars_from_env,
        state_dir,
    );
    command.envs(&env_diff.additions);
    for var in &env_diff.deletions {
        command.env_remove(var);
    }
}

/// Build environment variables from core context.
/// If project is provided, also includes service-related variables.
pub fn old_cli_envs(
    core: &ActivateCoreCtx,
    project: Option<&ActivateProjectCtx>,
) -> HashMap<&'static str, String> {
    let mut exports = HashMap::from([
        (
            FLOX_ACTIVE_ENVIRONMENTS_VAR,
            core.flox_active_environments.clone(),
        ),
        ("FLOX_PROMPT_COLOR_1", core.prompt_color_1.clone()),
        ("FLOX_PROMPT_COLOR_2", core.prompt_color_2.clone()),
        // Set `FLOX_PROMPT_ENVIRONMENTS` to the constructed prompt string,
        // which may be ""
        // This is used by set-prompt script, and tcsh in particular does not
        // tolerate references to undefined variables.
        (
            FLOX_PROMPT_ENVIRONMENTS_VAR,
            core.flox_prompt_environments.clone(),
        ),
        ("_FLOX_SET_PROMPT", core.set_prompt.to_string()),
        (
            // TODO: we should probably figure out a more consistent way to
            // pass this since it's also passed for `flox build`
            FLOX_RUNTIME_DIR_VAR,
            core.flox_runtime_dir.clone(),
        ),
        (
            "_FLOX_ENV_CUDA_DETECTION",
            core.flox_env_cuda_detection.clone(),
        ),
        // This is user-facing and documented
        (
            FLOX_ACTIVATE_START_SERVICES_VAR,
            project
                .is_some_and(|p| !p.services_to_start.is_empty())
                .to_string(),
        ),
    ]);

    if let Some(project) = project {
        exports.insert(
            FLOX_ENV_LOG_DIR_VAR,
            project.flox_env_log_dir.to_string_lossy().to_string(),
        );
        exports.insert(
            FLOX_SERVICES_SOCKET_VAR,
            project.flox_services_socket.to_string_lossy().to_string(),
        );
    }

    exports.extend(default_nix_env_vars());

    exports
}

/// Prior to the refactor, these options were passed by the CLI to the activate
/// script
fn add_old_cli_options(command: &mut Command, context: &ActivateCtx) {
    command
        .arg("--env-project")
        .arg(context.project.env_project.to_string_lossy().to_string());
    command
        .arg("--env-cache")
        .arg(context.core.env_cache.to_string_lossy().to_string());
    command
        .arg("--env-description")
        .arg(context.core.env_description.clone());

    // Pass down the activation mode
    command.arg("--mode").arg(context.core.mode.to_string());
}

/// Options parsed by getopt that are only used by the activate script
fn add_activate_script_options(command: &mut Command, state_dir: &Path) {
    command.args(["--start-state-dir", &state_dir.to_string_lossy()]);
}

/// Prior to the refactor, these variables were exported in the activate script
// TODO: we still use std::env::var in this function,
// so we should either drop those uses and get those vars in VarsFromEnvironment,
// or we should completely drop VarsFromEnvironment .
pub fn add_old_activate_script_exports(
    command: &mut Command,
    core: &ActivateCoreCtx,
    project: Option<&ActivateProjectCtx>,
    subsystem_verbosity: u32,
    vars_from_environment: VarsFromEnvironment,
    state_dir: &Path,
) {
    let mut removals = Vec::new();
    let mut exports = HashMap::from([
        ("_flox_activate_tracelevel", subsystem_verbosity.to_string()),
        // Propagate required variables that are documented as exposed.
        ("FLOX_ENV", core.env.clone()),
        (
            "FLOX_ENV_CACHE",
            core.env_cache.to_string_lossy().to_string(),
        ),
        ("FLOX_ENV_DESCRIPTION", core.env_description.clone()),
        (
            "_FLOX_START_STATE_DIR",
            state_dir.to_string_lossy().to_string(),
        ),
        // These are used by various scripts...custom ZDOTDIR files, set-prompt,
        // .tcshrc
        (
            "_flox_activate_tracer",
            activate_tracer(&core.interpreter_path),
        ),
        (
            "_activate_d",
            core.interpreter_path
                .join("activate.d")
                .to_string_lossy()
                .to_string(),
        ),
    ]);
    // Propagate optional variables that are documented as exposed.
    // NB: `generate_*_start_commands()` performs the same logic except for zsh.
    if let Some(project) = project {
        exports.insert(
            "FLOX_ENV_PROJECT",
            project.env_project.to_string_lossy().to_string(),
        );
    } else {
        removals.push("FLOX_ENV_PROJECT");
    }

    exports.extend(fixed_vars_to_export(&core.env, vars_from_environment));

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
