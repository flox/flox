use std::borrow::Cow;
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;

use anyhow::Result;
use itertools::Itertools;
use shell_gen::{
    GenerateShell,
    Shell,
    set_exported_unexpanded,
    set_unexported_unexpanded,
    source_file,
    unset,
};

use crate::gen_rc::RM;
use crate::start_diff::StartDiff;

/// Arguments for generating zsh startup commands
#[derive(Debug, Clone)]
pub struct ZshStartupArgs {
    pub flox_activate_tracelevel: u32,
    pub activate_d: PathBuf,
    pub flox_env: PathBuf,
    pub flox_env_cache: Option<PathBuf>,
    pub flox_env_project: Option<PathBuf>,
    pub flox_env_description: Option<String>,
    pub is_in_place: bool,
    pub clean_up: Option<PathBuf>,
    pub set_prompt: bool,
}

pub fn generate_zsh_startup_commands(
    args: &ZshStartupArgs,
    start_diff: &StartDiff,
    single_sets: &HashMap<String, String>,
    double_sets: &HashMap<String, String>,
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

    // For non-in-place activations, these were set as environment variables
    // prior to exec'ing
    if args.is_in_place {
        for (k, v) in single_sets.iter().sorted_by_key(|(k, _)| *k) {
            stmts.push(set_exported_unexpanded(k, v));
        }
    }
    for (k, v) in double_sets.iter().sorted_by_key(|(k, _)| *k) {
        stmts.push(set_exported_unexpanded(k, v));
    }

    // Restore environment variables set in the previous initialization.
    start_diff.generate_statements(&mut stmts);
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

    // Set the prompt if we're in an interactive shell.
    if args.set_prompt {
        let set_prompt_path = args.activate_d.join("set-prompt.zsh");
        stmts.push(
            format!(
                "if [[ -o interactive ]]; then source '{}'; fi;",
                set_prompt_path.display()
            )
            .to_stmt(),
        );
    }

    // The zsh script depends on these variables
    // unset immediately after sourcing to avoid leaking variables
    stmts.push(unset("_FLOX_ENV"));
    stmts.push(unset("_FLOX_ENV_CACHE"));
    stmts.push(unset("_FLOX_ENV_PROJECT"));
    stmts.push(unset("_FLOX_ENV_DESCRIPTION"));

    if let Some(path) = args.clean_up.as_ref() {
        let path_str = path.to_string_lossy();
        let escaped_path = shell_escape::escape(Cow::Borrowed(path_str.as_ref()));
        stmts.push(format!("{RM} {};", escaped_path).to_stmt());
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
    use expect_test::expect;

    use super::*;

    // NOTE: For these `expect!` tests, run unit tests with `UPDATE_EXPECT=1`
    //  to have it automatically update the expected value when the implementation
    //  changes.

    fn basic_args(
        is_in_place: bool,
    ) -> (
        ZshStartupArgs,
        HashMap<String, String>,
        HashMap<String, String>,
    ) {
        let args = ZshStartupArgs {
            flox_activate_tracelevel: 3,
            activate_d: PathBuf::from("/activate_d"),
            flox_env: "/flox_env".into(),
            flox_env_cache: Some("/flox_env_cache".into()),
            flox_env_project: Some("/flox_env_project".into()),
            flox_env_description: Some("env_description".to_string()),
            is_in_place,
            clean_up: Some("/path/to/rc/file".into()),
            set_prompt: true,
        };
        let single_sets = HashMap::from([
            ("SINGLE_B".to_string(), "single_b".to_string()),
            ("SINGLE_A".to_string(), "single_a".to_string()),
        ]);
        let double_sets = HashMap::from([("DOUBLE_X".to_string(), "double_x".to_string())]);
        (args, single_sets, double_sets)
    }

    fn render(
        args: &ZshStartupArgs,
        single_sets: &HashMap<String, String>,
        double_sets: &HashMap<String, String>,
    ) -> String {
        let additions = HashMap::from([
            ("QUOTED_VAR".to_string(), "QUOTED'VALUE".to_string()),
            ("ADDED_VAR".to_string(), "ADDED_VALUE".to_string()),
        ]);
        let deletions = vec!["DELETED_VAR".to_string()];
        let start_diff = StartDiff::from_parts(additions, deletions);
        let mut buf = Vec::new();
        generate_zsh_startup_commands(args, &start_diff, single_sets, double_sets, &mut buf)
            .unwrap();
        String::from_utf8_lossy(&buf).into_owned()
    }

    #[test]
    fn test_generate_zsh_startup_commands_subprocess() {
        let (args, single_sets, double_sets) = basic_args(false);
        let output = render(&args, &single_sets, &double_sets);
        let (main_output, last_line) = output
            .strip_suffix('\n')
            .unwrap()
            .rsplit_once('\n')
            .unwrap();
        assert_eq!(last_line, format!("{RM} /path/to/rc/file;"));
        expect![[r#"
            typeset -g _flox_activate_tracelevel=3;
            typeset -g _activate_d=/activate_d;
            export DOUBLE_X=double_x;
            export ADDED_VAR=ADDED_VALUE;
            export QUOTED_VAR='QUOTED'\''VALUE';
            unset DELETED_VAR;
            typeset -g _FLOX_ENV=/flox_env;
            typeset -g _FLOX_ENV_CACHE=/flox_env_cache;
            typeset -g _FLOX_ENV_PROJECT=/flox_env_project;
            typeset -g _FLOX_ENV_DESCRIPTION=env_description;
            source /activate_d/zsh;
            if [[ -o interactive ]]; then source '/activate_d/set-prompt.zsh'; fi;
            unset _FLOX_ENV;
            unset _FLOX_ENV_CACHE;
            unset _FLOX_ENV_PROJECT;
            unset _FLOX_ENV_DESCRIPTION;"#]]
        .assert_eq(main_output);
    }

    #[test]
    fn test_generate_zsh_startup_commands_in_place() {
        let (args, single_sets, double_sets) = basic_args(true);
        let output = render(&args, &single_sets, &double_sets);
        let (main_output, last_line) = output
            .strip_suffix('\n')
            .unwrap()
            .rsplit_once('\n')
            .unwrap();
        assert_eq!(last_line, format!("{RM} /path/to/rc/file;"));
        expect![[r#"
            typeset -g _flox_activate_tracelevel=3;
            typeset -g _activate_d=/activate_d;
            export SINGLE_A=single_a;
            export SINGLE_B=single_b;
            export DOUBLE_X=double_x;
            export ADDED_VAR=ADDED_VALUE;
            export QUOTED_VAR='QUOTED'\''VALUE';
            unset DELETED_VAR;
            typeset -g _FLOX_ENV=/flox_env;
            typeset -g _FLOX_ENV_CACHE=/flox_env_cache;
            typeset -g _FLOX_ENV_PROJECT=/flox_env_project;
            typeset -g _FLOX_ENV_DESCRIPTION=env_description;
            source /activate_d/zsh;
            if [[ -o interactive ]]; then source '/activate_d/set-prompt.zsh'; fi;
            unset _FLOX_ENV;
            unset _FLOX_ENV_CACHE;
            unset _FLOX_ENV_PROJECT;
            unset _FLOX_ENV_DESCRIPTION;"#]]
        .assert_eq(main_output);
    }
}
