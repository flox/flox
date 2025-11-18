use std::io::Write;
use std::path::PathBuf;

use anyhow::Result;
use shell_gen::{GenerateShell, Shell, set_exported_unexpanded, source_file, unset};

use crate::env_diff::EnvDiff;

/// Arguments for generating bash startup commands
#[derive(Debug, Clone)]
pub struct BashStartupArgs {
    pub flox_activate_tracelevel: u32,
    pub activate_d: PathBuf,
    pub flox_env: PathBuf,
    pub flox_env_cache: Option<PathBuf>,
    pub flox_env_project: Option<PathBuf>,
    pub flox_env_description: Option<String>,
    pub is_in_place: bool,
    pub clean_up: Option<PathBuf>,

    // Some(_) if it exists, None otherwise
    pub bashrc_path: Option<PathBuf>,
    pub flox_activate_tracer: String,
    pub flox_sourcing_rc: bool,
    pub flox_activations: PathBuf,
}

pub fn generate_bash_startup_commands(
    args: &BashStartupArgs,
    env_diff: &EnvDiff,
    writer: &mut impl Write,
) -> Result<()> {
    let mut stmts = vec![];

    // Enable trace mode if requested
    if args.flox_activate_tracelevel >= 2 {
        stmts.push("set -x".to_stmt());
    }

    // Only `Some` if it was determined to exist by the caller
    let should_source = args.bashrc_path.is_some() && !args.is_in_place && !args.flox_sourcing_rc;

    if should_source {
        stmts.push(set_exported_unexpanded("_flox_sourcing_rc", "true"));
        stmts.push(source_file(args.bashrc_path.as_ref().unwrap()));
        stmts.push(unset("_flox_sourcing_rc"));
    }

    // Restore environment variables set in the previous bash initialization.
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
        "_flox_activate_tracer",
        &args.flox_activate_tracer,
    ));

    // Set the prompt if we're in an interactive shell.
    let set_prompt_path = args.activate_d.join("set-prompt.bash");
    stmts.push(
        format!(
            "if [ -t 1 ]; then source '{}'; fi;",
            set_prompt_path.display()
        )
        .to_stmt(),
    );

    // We already customized the PATH and MANPATH, but the user and system
    // dotfiles may have changed them, so finish by doing this again.
    // Use generation time _FLOX_ENV because we want to guarantee we activate the
    // environment we think we're activating. Use runtime FLOX_ENV_DIRS to allow
    // RC files to perform activations.
    stmts.push(format!(
        r#"eval "$('{}' set-env-dirs --shell bash --flox-env "{}" --env-dirs "${{FLOX_ENV_DIRS:-}}")";"#,
        args.flox_activations.display(),
        args.flox_env.display()
    ).to_stmt());

    stmts.push(format!(
        r#"eval "$('{}' fix-paths --shell bash --env-dirs "$FLOX_ENV_DIRS" --path "$PATH" --manpath "${{MANPATH:-}}")";"#,
        args.flox_activations.display()
    ).to_stmt());

    stmts.push(format!(
        r#"eval "$('{}' profile-scripts --shell bash --already-sourced-env-dirs "${{_FLOX_SOURCED_PROFILE_SCRIPTS:-}}" --env-dirs "${{FLOX_ENV_DIRS:-}}")";"#,
        args.flox_activations.display()
    ).to_stmt());

    // Disable command hashing to allow for newly installed flox packages
    // to be found immediately. We do this as the very last thing because
    // python venv activations can otherwise return nonzero return codes
    // when attempting to invoke `hash -r`.
    stmts.push("set +h".to_stmt());

    // Disable trace mode if it was enabled
    if args.flox_activate_tracelevel >= 2 {
        stmts.push("set +x".to_stmt());
    }

    if let Some(path) = args.clean_up.as_ref() {
        stmts.push(format!("rm '{}';", path.display()).to_stmt());
    }

    // N.B. the output of these scripts may be eval'd with backticks which have
    // the effect of removing newlines from the output, so we must ensure that
    // the output is a valid shell script fragment when represented on a single line.
    for stmt in stmts {
        stmt.generate_with_newline(Shell::Bash, writer)?;
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
    fn test_generate_bash_startup_commands_basic() {
        let additions = {
            let mut map = HashMap::new();
            map.insert("ADDED_VAR".to_string(), "ADDED_VALUE".to_string());
            map
        };
        let deletions = vec!["DELETED_VAR".to_string()];
        let env_diff = EnvDiff::from_parts(additions, deletions);
        let args = BashStartupArgs {
            flox_activate_tracelevel: 3,
            activate_d: PathBuf::from("/activate_d"),
            flox_env: "/flox_env".into(),
            flox_env_cache: Some("/flox_env_cache".into()),
            flox_env_project: Some("/flox_env_project".into()),
            flox_env_description: Some("env_description".to_string()),
            is_in_place: false,
            bashrc_path: Some(PathBuf::from("/home/user/.bashrc")),
            flox_sourcing_rc: false,
            flox_activate_tracer: "TRACER".into(),
            flox_activations: PathBuf::from("/flox_activations"),
            clean_up: Some("/path/to/rc/file".into()),
        };
        let mut buf = Vec::new();
        generate_bash_startup_commands(&args, &env_diff, &mut buf).unwrap();
        let output = String::from_utf8_lossy(&buf);
        expect![[r#"
            set -x
            export _flox_sourcing_rc='true';
            source '/home/user/.bashrc';
            unset _flox_sourcing_rc;
            export ADDED_VAR='ADDED_VALUE';
            unset DELETED_VAR;
            export FLOX_ENV='/flox_env';
            export FLOX_ENV_CACHE='/flox_env_cache';
            export FLOX_ENV_PROJECT='/flox_env_project';
            export FLOX_ENV_DESCRIPTION='env_description';
            export _flox_activate_tracer='TRACER';
            if [ -t 1 ]; then source '/activate_d/set-prompt.bash'; fi;
            eval "$('/flox_activations' set-env-dirs --shell bash --flox-env "/flox_env" --env-dirs "${FLOX_ENV_DIRS:-}")";
            eval "$('/flox_activations' fix-paths --shell bash --env-dirs "$FLOX_ENV_DIRS" --path "$PATH" --manpath "${MANPATH:-}")";
            eval "$('/flox_activations' profile-scripts --shell bash --already-sourced-env-dirs "${_FLOX_SOURCED_PROFILE_SCRIPTS:-}" --env-dirs "${FLOX_ENV_DIRS:-}")";
            set +h
            set +x
            rm '/path/to/rc/file';
        "#]].assert_eq(&output);
    }
}
