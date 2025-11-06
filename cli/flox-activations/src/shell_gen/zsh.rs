use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};

use crate::shell_gen::Shell;
use crate::shell_gen::capture::ExportEnvDiff;

/// Arguments for generating zsh startup commands
pub struct ZshStartupArgs {
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

pub fn generate_zsh_startup_commands(
    args: &ZshStartupArgs,
    export_env_diff: &ExportEnvDiff,
) -> Result<String> {
    let mut commands = Vec::new();

    // Enable trace mode if requested
    if args.flox_activate_tracelevel >= 2 {
        commands.push("set -x".to_string());
    }

    // Restore environment variables set in the previous bash initialization.
    // Read del.env and add.env files
    commands.append(&mut export_env_diff.generate_commands(Shell::Zsh));

    // Propagate required variables that are documented as exposed.
    commands.push(Shell::Zsh.export_var("FLOX_ENV", &args.flox_env));

    // Propagate optional variables that are documented as exposed.
    if let Some(flox_env_cache) = &args.flox_env_cache {
        commands.push(Shell::Zsh.export_var("FLOX_ENV_CACHE", &flox_env_cache));
    } else {
        commands.push("unset FLOX_ENV_CACHE".to_string());
    }

    if let Some(flox_env_project) = &args.flox_env_project {
        commands.push(Shell::Zsh.export_var("FLOX_ENV_PROJECT", flox_env_project));
    } else {
        commands.push("unset FLOX_ENV_PROJECT".to_string());
    }

    if let Some(description) = &args.flox_env_description {
        commands.push(Shell::Zsh.export_var("FLOX_ENV_DESCRIPTION", description));
    } else {
        commands.push("unset FLOX_ENV_DESCRIPTION".to_string());
    }

    // Export the value of $_activate_d to the environment.
    commands.push(Shell::Zsh.export_var("_activate_d", &args.activate_d.display().to_string()));

    // Set _flox_activate_tracelevel for benefit of zsh script.
    commands.push(Shell::Zsh.export_var(
        "_flox_activate_tracelevel",
        &args.flox_activate_tracelevel.to_string(),
    ));

    // Export the value of $_flox_activate_tracer to the environment.
    commands.push(Shell::Zsh.export_var("_flox_activate_tracer", &args.flox_activate_tracer));

    commands.push("true not setting _flox_activations".to_string()); // DELETEME FOR DEBUGGING

    // Zsh isn't like the other shells in that initialization happens in a set
    // of scripts found in ZDOTDIR, so it's not quite so straightforward as to
    // simply generate a single set of commands to be sourced. Most of the heavy
    // lifting is done by the `zsh` script sourced by the following command.
    commands.push(format!(
        "source '{}/zsh'",
        &args.activate_d.display().to_string()
    ));

    /*
        // Our ZDOTDIR startup files source user RC files that may modify FLOX_ENV_DIRS,
        // and then _flox_env_helper may fix it up.
        // If this happens, we want to respect those modifications,
        // so we use FLOX_ENV_DIRS from the environment
        // Only source profile scripts for the current environment when activating from
        // an RC file because other environments will source their profile scripts
        // later in the nesting chain.
        commands.push(r#"if [ -n "${_flox_sourcing_rc:-}" ]; then profile_script_dirs="$FLOX_ENV"; else profile_script_dirs="$FLOX_ENV_DIRS"; fi"#.to_string());
        commands.push(r#"echo HI DAD PID $$ _FLOX_SOURCED_PROFILE_SCRIPTS is $_FLOX_SOURCED_PROFILE_SCRIPTS"#.to_string()); // DELETEME FOR DEBUGGING
        commands.push(format!(r#"if [ -z "${{FLOX_NOPROFILE:-}}" ]; then eval "$('{}' profile-scripts --shell zsh --already-sourced-env-dirs "${{_FLOX_SOURCED_PROFILE_SCRIPTS:-}}" --env-dirs "$profile_script_dirs")"; fi"#,
            args.flox_activations.display()
        ));
        commands.push(format!(r#"eval "$('{}' profile-scripts --shell zsh --already-sourced-env-dirs "${{_FLOX_SOURCED_PROFILE_SCRIPTS:-}}" --env-dirs "$profile_script_dirs")""#,
            args.flox_activations.display()
        ));
    */

    // Disable command hashing to allow for newly installed flox packages
    // to be found immediately. We do this as the very last thing because
    // python venv activations can otherwise return nonzero return codes
    // when attempting to invoke `hash -r`.
    commands.push("setopt nohashcmds".to_string());
    commands.push("setopt nohashdirs".to_string());

    // Disable trace mode if it was enabled
    if args.flox_activate_tracelevel >= 2 {
        commands.push("set +x".to_string());
    }

    // N.B. the output of these scripts may be eval'd with backticks which have
    // the effect of removing newlines from the output, so we must ensure that
    // the output is a valid shell script fragment when represented on a single line.
    commands.push("".to_string()); // ensure there's a trailing newline
    let mut joined = commands.join(";\n");
    Ok(joined)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_zsh_startup_commands_basic() {}
}
