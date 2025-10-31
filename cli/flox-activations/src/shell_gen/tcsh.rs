use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};

use crate::shell_gen::Shell;
use crate::shell_gen::capture::ExportEnvDiff;

/// Arguments for generating tcsh startup commands
pub struct TcshStartupArgs {
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

pub fn generate_tcsh_startup_commands(
    args: &TcshStartupArgs,
    export_env_diff: &ExportEnvDiff,
) -> Result<String> {
    let mut commands = Vec::new();

    // Enable trace mode if requested
    if args.flox_activate_tracelevel >= 2 {
        commands.push("set verbose".to_string());
    }

    // The tcsh implementation will source our custom .tcshrc
    // which will then source the result of this script as $FLOX_TCSH_INIT_SCRIPT
    // after the normal configuration has been processed,
    // so there is no requirement to go back and source the user's own config
    // as we do in bash.

    // Restore environment variables set in the previous tcsh initialization.
    // Read del.env and add.env files
    commands.append(&mut export_env_diff.generate_commands(Shell::Tcsh));

    // Propagate required variables that are documented as exposed.
    commands.push(Shell::Tcsh.export_var("FLOX_ENV", &args.flox_env));

    // Propagate optional variables that are documented as exposed.
    if let Some(flox_env_cache) = &args.flox_env_cache {
        commands.push(Shell::Tcsh.export_var("FLOX_ENV_CACHE", &flox_env_cache));
    } else {
        commands.push("unset FLOX_ENV_CACHE".to_string());
    }

    if let Some(flox_env_project) = &args.flox_env_project {
        commands.push(Shell::Tcsh.export_var("FLOX_ENV_PROJECT", flox_env_project));
    } else {
        commands.push("unset FLOX_ENV_PROJECT".to_string());
    }

    if let Some(description) = &args.flox_env_description {
        commands.push(Shell::Tcsh.export_var("FLOX_ENV_DESCRIPTION", description));
    } else {
        commands.push("unset FLOX_ENV_DESCRIPTION".to_string());
    }

    // Export the value of $_activate_d to the environment.
    commands.push(Shell::Tcsh.export_var("_activate_d", &args.activate_d.display().to_string()));

    // Export the value of $_flox_activate_tracer to the environment.
    commands.push(Shell::Tcsh.export_var("_flox_activate_tracer", &args.flox_activate_tracer));

    commands.push("true not setting _flox_activations".to_string()); // DELETEME FOR DEBUGGING

    // Set the prompt if we're in an interactive shell.
    let set_prompt_path = args.activate_d.join("set-prompt.tcsh");
    commands.push(format!(
        "if ( $?tty ) then; source '{}'; endif",
        set_prompt_path.display()
    ));

    // We already customized the PATH and MANPATH, but the user and system
    // dotfiles may have changed them, so finish by doing this again.
    // Use generation time _FLOX_ENV because we want to guarantee we activate the
    // environment we think we're activating. Use runtime FLOX_ENV_DIRS to allow
    // RC files to perform activations.
    commands.push(r#"if (! $?FLOX_ENV_DIRS) setenv FLOX_ENV_DIRS "empty""#.to_string());

    commands.push(format!(
        r#"eval "`'{}' set-env-dirs --shell tcsh --flox-env '{}' --env-dirs $FLOX_ENV_DIRS:q`""#,
        args.flox_activations.display(),
        args.flox_env
    ));

    commands.push(r#"if (! $?MANPATH) setenv MANPATH "empty""#.to_string());

    commands.push(format!(
        r#"eval "`'{}' fix-paths --shell tcsh --env-dirs $FLOX_ENV_DIRS:q --path $PATH:q --manpath $MANPATH:q`""#,
        args.flox_activations.display()
    ));

    // Modern versions of tcsh support the ":Q" modifier for passing empty args
    // on the command line, but versions prior to 6.23 do not have a way to do
    // that, so to support these versions we will instead avoid passing the
    // --already-sourced-env-dirs argument altogether when there is no default
    // value to be passed.
    commands.push("set _already_sourced_args = ()".to_string());

    commands.push(
        r#"if ($?_FLOX_SOURCED_PROFILE_SCRIPTS) set _already_sourced_args = ( --already-sourced-env-dirs `echo $_FLOX_SOURCED_PROFILE_SCRIPTS:q` )"#.to_string()
    );

    commands.push(format!(
        r#"if (! $?FLOX_NOPROFILE) eval "`'{}' profile-scripts --shell tcsh --env-dirs $FLOX_ENV_DIRS:q $_already_sourced_args:q`""#,
        args.flox_activations.display()
    ));

    // Disable command hashing to allow for newly installed flox packages
    // to be found immediately. We do this as the very last thing because
    // python venv activations can otherwise return nonzero return codes
    // when attempting to invoke `hash -r`.
    commands.push("unhash".to_string());

    // Disable trace mode if it was enabled
    if args.flox_activate_tracelevel >= 2 {
        commands.push("unset verbose".to_string());
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
    fn test_generate_tcsh_startup_commands_basic() {}
}
