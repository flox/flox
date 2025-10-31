use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};

use crate::shell_gen::Shell;
use crate::shell_gen::capture::ExportEnvDiff;

/// Arguments for generating fish startup commands
pub struct FishStartupArgs {
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

pub fn generate_fish_startup_commands(
    args: &FishStartupArgs,
    export_env_diff: &ExportEnvDiff,
) -> Result<String> {
    let mut commands = Vec::new();

    // Enable trace mode if requested
    if args.flox_activate_tracelevel >= 2 {
        commands.push("set -gx fish_trace 1".to_string());
    }

    // The fish --init-command option allows us to source our startup
    // file after the normal configuration has been processed, so there
    // is no requirement to go back and source the user's own config
    // as we do in bash.

    // Restore environment variables set in the previous fish initialization.
    // Read del.env and add.env files
    commands.append(&mut export_env_diff.generate_commands(Shell::Fish));

    // Propagate required variables that are documented as exposed.
    commands.push(Shell::Fish.export_var("FLOX_ENV", &args.flox_env));

    // Propagate optional variables that are documented as exposed.
    if let Some(flox_env_cache) = &args.flox_env_cache {
        commands.push(Shell::Fish.export_var("FLOX_ENV_CACHE", &flox_env_cache));
    } else {
        commands.push("unset FLOX_ENV_CACHE".to_string());
    }

    if let Some(flox_env_project) = &args.flox_env_project {
        commands.push(Shell::Fish.export_var("FLOX_ENV_PROJECT", flox_env_project));
    } else {
        commands.push("unset FLOX_ENV_PROJECT".to_string());
    }

    if let Some(description) = &args.flox_env_description {
        commands.push(Shell::Fish.export_var("FLOX_ENV_DESCRIPTION", description));
    } else {
        commands.push("unset FLOX_ENV_DESCRIPTION".to_string());
    }

    commands.push("true not setting _activate_d".to_string()); // DELETEME FOR DEBUGGING

    // Export the value of $_flox_activate_tracer from the environment.
    commands.push(Shell::Fish.export_var("_flox_activate_tracer", &args.flox_activate_tracer));

    commands.push("true not setting _flox_activations".to_string()); // DELETEME FOR DEBUGGING

    // Set the prompt if we're in an interactive shell.
    let set_prompt_path = args.activate_d.join("set-prompt.fish");
    commands.push(format!(
        "if isatty 1; source '{}'; end",
        set_prompt_path.display()
    ));

    // We already customized the PATH and MANPATH, but the user and system
    // dotfiles may have changed them, so finish by doing this again.

    // fish doesn't have {foo:-} syntax, so we need to provide a temporary variable
    // (foo_with_default) that is either the runtime (not generation-time) value
    // or the string 'empty'.
    commands.push(
        r#"set -gx FLOX_ENV_DIRS (if set -q FLOX_ENV_DIRS; echo "$FLOX_ENV_DIRS"; else; echo empty; end)"#.to_string()
    );

    commands.push(format!(
        r#"{} set-env-dirs --shell fish --flox-env "{}" --env-dirs "$FLOX_ENV_DIRS" | source"#,
        args.flox_activations.display(),
        args.flox_env
    ));

    commands.push(
        r#"set -gx MANPATH (if set -q MANPATH; echo "$MANPATH"; else; echo empty; end)"#
            .to_string(),
    );

    commands.push(format!(
        r#"{} fix-paths --shell fish --env-dirs "$FLOX_ENV_DIRS" --path "$PATH" --manpath "$MANPATH" | source"#,
        args.flox_activations.display()
    ));

    commands.push(
        r#"set -g  _FLOX_SOURCED_PROFILE_SCRIPTS (if set -q _FLOX_SOURCED_PROFILE_SCRIPTS; echo "$_FLOX_SOURCED_PROFILE_SCRIPTS"; else; echo ""; end)"#.to_string()
    );

    commands.push(format!(
        r#"if set -q FLOX_NOPROFILE; true; else; {} profile-scripts --shell fish --already-sourced-env-dirs  "$_FLOX_SOURCED_PROFILE_SCRIPTS" --env-dirs "$FLOX_ENV_DIRS"; end | source"#,
        args.flox_activations.display()
    ));

    // fish does not use hashing in the same way bash does, so there's
    // nothing to be done here by way of that requirement.

    // Disable trace mode if it was enabled
    if args.flox_activate_tracelevel >= 2 {
        commands.push("set -gx fish_trace 0".to_string());
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
    fn test_generate_fish_startup_commands_basic() {}
}
