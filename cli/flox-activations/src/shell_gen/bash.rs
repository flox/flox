use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};

use crate::shell_gen::Shell;
use crate::shell_gen::capture::ExportEnvDiff;

/// Arguments for generating bash startup commands
pub struct BashStartupArgs {
    pub flox_activate_tracelevel: i32,
    pub activate_d: PathBuf,
    pub flox_env: String,
    pub flox_env_cache: Option<String>,
    pub flox_env_project: Option<String>,
    pub flox_env_description: Option<String>,
    pub is_in_place: bool,

    pub flox_sourcing_rc: bool,
    pub flox_activations: PathBuf,
    pub flox_activate_tracer: String,
}

pub fn generate_bash_startup_commands(
    args: &BashStartupArgs,
    export_env_diff: &ExportEnvDiff,
) -> Result<String> {
    let mut commands = Vec::new();

    // Enable trace mode if requested
    if args.flox_activate_tracelevel >= 2 {
        commands.push("set -x".to_string());
    }

    // We need to source the .bashrc file exactly once. We skip it for in-place
    // activations under the assumption that it has already been sourced by one
    // of the shells in the chain of ancestors UNLESS none of them were bash
    // and therefore .bashrc hasn't been sourced yet.
    let bashrc_path = if let Some(home_dir) = dirs::home_dir() {
        home_dir.join(".bashrc")
    } else {
        return Err(anyhow!("failed to get home directory"));
    };

    let should_source = bashrc_path.exists() && !args.is_in_place && !args.flox_sourcing_rc;

    if should_source {
        commands.push("export _flox_sourcing_rc=true".to_string());
        commands.push(format!("source '{}'", bashrc_path.display()));
        commands.push("unset _flox_sourcing_rc".to_string());
    }

    // Restore environment variables set in the previous bash initialization.
    // Read del.env and add.env files
    commands.append(&mut export_env_diff.generate_commands(Shell::Bash));

    // Propagate required variables that are documented as exposed.
    commands.push(Shell::Bash.export_var("FLOX_ENV", &args.flox_env));

    // Propagate optional variables that are documented as exposed.
    if let Some(flox_env_cache) = &args.flox_env_cache {
        commands.push(Shell::Bash.export_var("FLOX_ENV_CACHE", &flox_env_cache));
    } else {
        commands.push("unset FLOX_ENV_CACHE".to_string());
    }

    if let Some(flox_env_project) = &args.flox_env_project {
        commands.push(Shell::Bash.export_var("FLOX_ENV_PROJECT", flox_env_project));
    } else {
        commands.push("unset FLOX_ENV_PROJECT".to_string());
    }

    if let Some(description) = &args.flox_env_description {
        commands.push(Shell::Bash.export_var("FLOX_ENV_DESCRIPTION", description));
    } else {
        commands.push("unset FLOX_ENV_DESCRIPTION;".to_string());
    }

    // Export the value of $_flox_activate_tracer from the environment.
    commands.push(Shell::Bash.export_var("_flox_activate_tracer", &args.flox_activate_tracer));

    // Set the prompt if we're in an interactive shell.
    let set_prompt_path = args.activate_d.join("set-prompt.bash");
    commands.push(format!(
        "if [ -t 1 ]; then source '{}'; fi",
        set_prompt_path.display()
    ));

    // We already customized the PATH and MANPATH, but the user and system
    // dotfiles may have changed them, so finish by doing this again.
    // Use generation time _FLOX_ENV because we want to guarantee we activate the
    // environment we think we're activating. Use runtime FLOX_ENV_DIRS to allow
    // RC files to perform activations.
    commands.push(format!(
        r#"eval "$('{}' set-env-dirs --shell bash --flox-env "{}" --env-dirs "${{FLOX_ENV_DIRS:-}}")""#,
        args.flox_activations.display(),
        args.flox_env
    ));

    commands.push(format!(
        r#"eval "$('{}' fix-paths --shell bash --env-dirs "$FLOX_ENV_DIRS" --path "$PATH" --manpath "${{MANPATH:-}}")""#,
        args.flox_activations.display()
    ));

    commands.push(format!(
        r#"eval "$('{}' profile-scripts --shell bash --already-sourced-env-dirs "${{_FLOX_SOURCED_PROFILE_SCRIPTS:-}}" --env-dirs "${{FLOX_ENV_DIRS:-}}")""#,
        args.flox_activations.display()
    ));

    // Disable command hashing to allow for newly installed flox packages
    // to be found immediately. We do this as the very last thing because
    // python venv activations can otherwise return nonzero return codes
    // when attempting to invoke `hash -r`.
    commands.push("set +h".to_string());

    // Disable trace mode if it was enabled
    if args.flox_activate_tracelevel >= 2 {
        commands.push("set +x".to_string());
    }

    // N.B. the output of these scripts may be eval'd with backticks which have
    // the effect of removing newlines from the output, so we must ensure that
    // the output is a valid shell script fragment when represented on a single line.
    let mut joined = commands.join(";\n");
    joined.push(';');
    Ok(joined)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_bash_startup_commands_basic() {}
}
