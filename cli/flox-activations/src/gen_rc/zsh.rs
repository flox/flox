use std::io::Write;
use std::path::PathBuf;

use anyhow::Result;
use shell_gen::{GenerateShell, Shell, set_unexported_unexpanded, source_file};

use crate::env_diff::EnvDiff;

/// Arguments for generating zsh startup commands
#[derive(Debug, Clone)]
pub struct ZshStartupArgs {
    pub flox_activate_tracelevel: u32,
    pub activate_d: PathBuf,
    pub flox_env: PathBuf,
    pub flox_env_cache: Option<PathBuf>,
    pub flox_env_project: Option<PathBuf>,
    pub flox_env_description: Option<String>,
    pub clean_up: Option<PathBuf>,
}

pub fn generate_zsh_startup_commands(
    args: &ZshStartupArgs,
    env_diff: &EnvDiff,
    writer: &mut impl Write,
) -> Result<()> {
    let mut stmts = vec![];
    stmts.push(set_unexported_unexpanded(
        "_flox_activate_tracelevel",
        format!("{}", &args.flox_activate_tracelevel),
    ));
    stmts.push(set_unexported_unexpanded(
        "_activate_d",
        args.activate_d.display().to_string(),
    ));

    // Restore environment variables set in the previous initialization.
    env_diff.generate_statements(&mut stmts);
    // Propagate required variables that are documented as exposed.
    stmts.push(set_unexported_unexpanded(
        "_FLOX_ENV",
        args.flox_env.display().to_string(),
    ));
    if let Some(flox_env_cache) = &args.flox_env_cache {
        stmts.push(set_unexported_unexpanded(
            "_FLOX_ENV_CACHE",
            flox_env_cache.display().to_string(),
        ));
    }
    if let Some(flox_env_project) = &args.flox_env_project {
        stmts.push(set_unexported_unexpanded(
            "_FLOX_ENV_PROJECT",
            flox_env_project.display().to_string(),
        ));
    }
    if let Some(description) = &args.flox_env_description {
        stmts.push(set_unexported_unexpanded(
            "_FLOX_ENV_DESCRIPTION",
            description,
        ));
    }
    stmts.push(source_file(args.activate_d.join("zsh")));

    if let Some(path) = args.clean_up.as_ref() {
        stmts.push(format!("rm '{}';", path.display()).to_stmt());
    }

    // N.B. the output of these scripts may be eval'd with backticks which have
    // the effect of removing newlines from the output, so we must ensure that
    // the output is a valid shell script fragment when represented on a single line.
    for stmt in stmts {
        stmt.generate_with_newline(Shell::Zsh, writer)?;
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
    fn test_generate_zsh_startup_commands_basic() {
        let additions = {
            let mut map = HashMap::new();
            map.insert("QUOTED_VAR".to_string(), "QUOTED'VALUE".to_string());
            map.insert("ADDED_VAR".to_string(), "ADDED_VALUE".to_string());
            map
        };
        let deletions = vec!["DELETED_VAR".to_string()];
        let env_diff = EnvDiff::from_parts(additions, deletions);
        let args = ZshStartupArgs {
            flox_activate_tracelevel: 3,
            activate_d: PathBuf::from("/activate_d"),
            flox_env: "/flox_env".into(),
            flox_env_cache: Some("/flox_env_cache".into()),
            flox_env_project: Some("/flox_env_project".into()),
            flox_env_description: Some("env_description".to_string()),
            clean_up: Some("/path/to/rc/file".into()),
        };
        let mut buf = Vec::new();
        generate_zsh_startup_commands(&args, &env_diff, &mut buf).unwrap();
        let output = String::from_utf8_lossy(&buf);
        expect![[r#"
            typeset -g _flox_activate_tracelevel=3;
            typeset -g _activate_d=/activate_d;
            export ADDED_VAR=ADDED_VALUE;
            export QUOTED_VAR='QUOTED'\''VALUE';
            unset DELETED_VAR;
            typeset -g _FLOX_ENV=/flox_env;
            typeset -g _FLOX_ENV_CACHE=/flox_env_cache;
            typeset -g _FLOX_ENV_PROJECT=/flox_env_project;
            typeset -g _FLOX_ENV_DESCRIPTION=env_description;
            source '/activate_d/zsh';
            rm '/path/to/rc/file';
        "#]]
        .assert_eq(&output);
    }
}
