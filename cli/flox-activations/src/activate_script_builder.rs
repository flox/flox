use std::borrow::Cow;
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

use flox_core::activate::context::{ActivateCtx, AttachCtx, AttachProjectCtx};
use flox_core::activate::vars::FLOX_ACTIVE_ENVIRONMENTS_VAR;
use flox_core::util::default_nix_env_vars;
use is_executable::IsExecutable;
use itertools::Itertools;
use shell_gen::ShellWithPath;

use crate::cli::fix_paths::{fix_manpath_var, fix_path_var};
use crate::cli::set_env_dirs::fix_env_dirs_var;
use crate::env_diff::EnvDiff;
use crate::vars_from_env::VarsFromEnvironment;
pub const FLOX_PROMPT_ENVIRONMENTS_VAR: &str = "FLOX_PROMPT_ENVIRONMENTS";

pub const FLOX_ACTIVATE_START_SERVICES_VAR: &str = "FLOX_ACTIVATE_START_SERVICES";
pub const FLOX_ENV_DIRS_VAR: &str = "FLOX_ENV_DIRS";

pub(super) fn assemble_activate_command(
    context: &ActivateCtx,
    subsystem_verbosity: u32,
    vars_from_env: VarsFromEnvironment,
    start_state_dir: &Path,
) -> Command {
    let mut command = Command::new(context.attach_ctx.interpreter_path.join("activate"));
    command.envs(common_activation_envs(
        &context.attach_ctx,
        context.project_ctx.as_ref(),
    ));
    add_old_activate_script_exports(
        &mut command,
        &context.attach_ctx,
        context.project_ctx.as_ref(),
        subsystem_verbosity,
        vars_from_env,
    );
    add_activate_script_options(&mut command, context, start_state_dir);
    command
}

/// Set (and unset) environment variables needed to be activated.
pub fn apply_activation_env(
    command: &mut Command,
    context: &AttachCtx,
    project: Option<&AttachProjectCtx>,
    subsystem_verbosity: u32,
    vars_from_env: VarsFromEnvironment,
    env_diff: &EnvDiff,
) {
    command.envs(common_activation_envs(context, project));
    add_old_activate_script_exports(
        command,
        context,
        project,
        subsystem_verbosity,
        vars_from_env,
    );
    command.envs(&env_diff.additions);
    for var in &env_diff.deletions {
        command.env_remove(var);
    }
}

/// Build environment variables exported before invoking activate script.
fn common_activation_envs(
    context: &AttachCtx,
    project: Option<&AttachProjectCtx>,
) -> HashMap<&'static str, String> {
    let mut exports = HashMap::from([
        (
            FLOX_ACTIVE_ENVIRONMENTS_VAR,
            context.flox_active_environments.clone(),
        ),
        ("FLOX_PROMPT_COLOR_1", context.prompt_color_1.clone()),
        ("FLOX_PROMPT_COLOR_2", context.prompt_color_2.clone()),
        // Set `FLOX_PROMPT_ENVIRONMENTS` to the constructed prompt string,
        // which may be ""
        // This is used by set-prompt script, and tcsh in particular does not
        // tolerate references to undefined variables.
        (
            FLOX_PROMPT_ENVIRONMENTS_VAR,
            context.flox_prompt_environments.clone(),
        ),
        ("_FLOX_SET_PROMPT", context.set_prompt.to_string()),
        (
            "_FLOX_ENV_CUDA_DETECTION",
            context.flox_env_cuda_detection.clone(),
        ),
        // This is user-facing and documented
        (
            FLOX_ACTIVATE_START_SERVICES_VAR,
            project
                .is_some_and(|p| !p.services_to_start.is_empty())
                .to_string(),
        ),
    ]);

    exports.extend(default_nix_env_vars());

    exports
}

/// Render shell exports for variables that are set before invoking attach.
pub fn render_common_activation_envs(
    shell: &ShellWithPath,
    context: &AttachCtx,
    project: Option<&AttachProjectCtx>,
) -> String {
    common_activation_envs(context, project)
        .iter()
        .map(|(key, value)| (key, shell_escape::escape(Cow::Borrowed(value))))
        .map(|(key, value)| match shell {
            ShellWithPath::Bash(_) => format!("export {key}={value};"),
            ShellWithPath::Fish(_) => format!("set -gx {key} {value};"),
            ShellWithPath::Tcsh(_) => format!("setenv {key} {value};"),
            ShellWithPath::Zsh(_) => format!("export {key}={value};"),
        })
        .join("\n")
}

/// Options parsed by getopt in the activate script
fn add_activate_script_options(
    command: &mut Command,
    context: &ActivateCtx,
    start_state_dir: &Path,
) {
    command.arg("--env").arg(&context.attach_ctx.env);

    // Pass down the activation mode
    command.arg("--mode").arg(context.mode.to_string());

    command.args(["--start-state-dir", &start_state_dir.to_string_lossy()]);
}

/// Prior to the refactor, these variables were exported in the activate script
// TODO: we still use std::env::var in this function,
// so we should either drop those uses and get those vars in VarsFromEnvironment,
// or we should completely drop VarsFromEnvironment .
fn add_old_activate_script_exports(
    command: &mut Command,
    context: &AttachCtx,
    project: Option<&AttachProjectCtx>,
    subsystem_verbosity: u32,
    vars_from_environment: VarsFromEnvironment,
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
    if let Some(project) = project {
        exports.insert(
            "FLOX_ENV_PROJECT",
            project.env_project.to_string_lossy().to_string(),
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
    let new_path = fix_path_var(
        &new_flox_env_dirs,
        &vars_from_environment.path.unwrap_or("".to_string()),
    );
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
