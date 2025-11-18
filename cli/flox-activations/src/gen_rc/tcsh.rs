use std::io::Write;
use std::path::PathBuf;

use anyhow::Result;
use shell_gen::{GenerateShell, Shell, set_exported_unexpanded, unset};

use crate::env_diff::EnvDiff;

/// Arguments for generating tcsh startup commands
#[derive(Debug, Clone)]
pub struct TcshStartupArgs {
    pub flox_activate_tracelevel: u32,
    pub activate_d: PathBuf,
    pub flox_env: PathBuf,
    pub flox_env_cache: Option<PathBuf>,
    pub flox_env_project: Option<PathBuf>,
    pub flox_env_description: Option<String>,
    pub is_in_place: bool,
    pub clean_up: Option<PathBuf>,

    pub flox_activate_tracer: String,
    pub flox_sourcing_rc: bool,
    pub flox_activations: PathBuf,
}

// N.B. the output of these scripts may be eval'd with backticks which have
// the effect of removing newlines from the output, so we must ensure that
// the output is a valid shell script fragment when represented on a single line.
pub fn generate_tcsh_startup_commands(
    args: &TcshStartupArgs,
    env_diff: &EnvDiff,
    writer: &mut impl Write,
) -> Result<()> {
    let mut stmts = vec![];

    // Enable trace mode if requested
    if args.flox_activate_tracelevel >= 2 {
        stmts.push("set verbose".to_stmt());
    }

    // Restore environment variables set in the previous tcsh initialization.
    env_diff.generate_statements(&mut stmts);

    // Propagate required variables that are documented as exposed.
    stmts.push(set_exported_unexpanded(
        "FLOX_ENV",
        args.flox_env.display().to_string(),
    ));

    // Propagate optional variables that are documented as exposed.
    if let Some(flox_env_cache) = &args.flox_env_cache {
        stmts.push(set_exported_unexpanded(
            "FLOX_ENV_CACHE",
            flox_env_cache.display().to_string(),
        ));
    } else {
        stmts.push(unset("FLOX_ENV_CACHE"));
    }

    if let Some(flox_env_project) = &args.flox_env_project {
        stmts.push(set_exported_unexpanded(
            "FLOX_ENV_PROJECT",
            flox_env_project.display().to_string(),
        ));
    } else {
        stmts.push(unset("FLOX_ENV_PROJECT"));
    }

    if let Some(description) = &args.flox_env_description {
        stmts.push(set_exported_unexpanded("FLOX_ENV_DESCRIPTION", description));
    } else {
        stmts.push(unset("FLOX_ENV_DESCRIPTION"));
    }

    stmts.push(set_exported_unexpanded(
        "_activate_d",
        args.activate_d.display().to_string(),
    ));
    stmts.push(set_exported_unexpanded(
        "_flox_activations",
        args.flox_activations.display().to_string(),
    ));

    stmts.push(set_exported_unexpanded(
        "_flox_activate_tracer",
        &args.flox_activate_tracer,
    ));

    // Set the prompt if we're in an interactive shell.
    let set_prompt_path = args.activate_d.join("set-prompt.tcsh");
    stmts.push(
        format!(
            "if ( $?tty ) then; source '{}'; endif;",
            set_prompt_path.display()
        )
        .to_stmt(),
    );

    // We already customized the PATH and MANPATH, but the user and system
    // dotfiles may have changed them, so finish by doing this again.
    // Use generation time _FLOX_ENV because we want to guarantee we activate the
    // environment we think we're activating. Use runtime FLOX_ENV_DIRS to allow
    // RC files to perform activations.
    stmts.push(r#"if (! $?FLOX_ENV_DIRS) setenv FLOX_ENV_DIRS "empty";"#.to_stmt());

    stmts.push(format!(
        r#"eval "`'{}' set-env-dirs --shell tcsh --flox-env '{}' --env-dirs $FLOX_ENV_DIRS:q`";"#,
        args.flox_activations.display(),
        args.flox_env.display(),
    ).to_stmt());

    stmts.push(r#"if (! $?MANPATH) setenv MANPATH "empty";"#.to_stmt());

    stmts.push(format!(
        r#"eval "`'{}' fix-paths --shell tcsh --env-dirs $FLOX_ENV_DIRS:q --path $PATH:q --manpath $MANPATH:q`";"#,
        args.flox_activations.display()
    ).to_stmt());

    // Modern versions of tcsh support the ":Q" modifier for passing empty args
    // on the command line, but versions prior to 6.23 do not have a way to do
    // that, so to support these versions we will instead avoid passing the
    // --already-sourced-env-dirs argument altogether when there is no default
    // value to be passed.
    stmts.push("set _already_sourced_args = ();".to_stmt());

    stmts.push(
        r#"if ($?_FLOX_SOURCED_PROFILE_SCRIPTS) set _already_sourced_args = ( --already-sourced-env-dirs `echo $_FLOX_SOURCED_PROFILE_SCRIPTS:q` );"#.to_stmt()
    );

    stmts.push(format!(
        r#"eval "`'{}' profile-scripts --shell tcsh --env-dirs $FLOX_ENV_DIRS:q $_already_sourced_args:q`";"#,
        args.flox_activations.display()
    ).to_stmt());

    // Disable command hashing to allow for newly installed flox packages
    // to be found immediately. We do this as the very last thing because
    // python venv activations can otherwise return nonzero return codes
    // when attempting to invoke `hash -r`.
    stmts.push("unhash;".to_stmt());

    // Disable trace mode if it was enabled
    if args.flox_activate_tracelevel >= 2 {
        stmts.push("unset verbose;".to_stmt());
    }

    if let Some(path) = args.clean_up.as_ref() {
        stmts.push(format!("rm '{}';", path.display()).to_stmt());
    }

    for stmt in stmts {
        stmt.generate_with_newline(Shell::Tcsh, writer)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use expect_test::expect;

    use super::*;

    // NOTE: For these `expect!` tests, run unit tests with `UPDATE_EXPECT=1`
    //  to have it automatically update the expected value when the implementation
    //  changes.

    #[test]
    fn test_generate_tcsh_startup_commands_basic() {
        let additions = {
            let mut map = HashMap::new();
            map.insert("ADDED_VAR".to_string(), "ADDED_VALUE".to_string());
            map
        };
        let deletions = vec!["DELETED_VAR".to_string()];
        let env_diff = EnvDiff::from_parts(additions, deletions);
        let args = TcshStartupArgs {
            flox_activate_tracelevel: 3,
            activate_d: PathBuf::from("/activate_d"),
            flox_env: "/flox_env".into(),
            flox_env_cache: Some("/flox_env_cache".into()),
            flox_env_project: Some("/flox_env_project".into()),
            flox_env_description: Some("env_description".to_string()),
            is_in_place: false,
            flox_sourcing_rc: false,
            flox_activate_tracer: "TRACER".into(),
            flox_activations: PathBuf::from("/flox_activations"),
            clean_up: Some("/path/to/rc/file".into()),
        };
        let mut buf = Vec::new();
        generate_tcsh_startup_commands(&args, &env_diff, &mut buf).unwrap();
        let output = String::from_utf8_lossy(&buf);
        expect![[r#"
            set verbose
            setenv ADDED_VAR 'ADDED_VALUE';
            unsetenv DELETED_VAR;
            setenv FLOX_ENV '/flox_env';
            setenv FLOX_ENV_CACHE '/flox_env_cache';
            setenv FLOX_ENV_PROJECT '/flox_env_project';
            setenv FLOX_ENV_DESCRIPTION 'env_description';
            setenv _activate_d '/activate_d';
            setenv _flox_activations '/flox_activations';
            setenv _flox_activate_tracer 'TRACER';
            if ( $?tty ) then; source '/activate_d/set-prompt.tcsh'; endif;
            if (! $?FLOX_ENV_DIRS) setenv FLOX_ENV_DIRS "empty";
            eval "`'/flox_activations' set-env-dirs --shell tcsh --flox-env '/flox_env' --env-dirs $FLOX_ENV_DIRS:q`";
            if (! $?MANPATH) setenv MANPATH "empty";
            eval "`'/flox_activations' fix-paths --shell tcsh --env-dirs $FLOX_ENV_DIRS:q --path $PATH:q --manpath $MANPATH:q`";
            set _already_sourced_args = ();
            if ($?_FLOX_SOURCED_PROFILE_SCRIPTS) set _already_sourced_args = ( --already-sourced-env-dirs `echo $_FLOX_SOURCED_PROFILE_SCRIPTS:q` );
            eval "`'/flox_activations' profile-scripts --shell tcsh --env-dirs $FLOX_ENV_DIRS:q $_already_sourced_args:q`";
            unhash;
            unset verbose;
            rm '/path/to/rc/file';
        "#]].assert_eq(&output);
    }
}
