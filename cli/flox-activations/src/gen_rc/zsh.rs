use std::borrow::Cow;
use std::io::Write;
use std::path::PathBuf;

use anyhow::Result;
use shell_gen::{GenerateShell, Shell, set_unexported_unexpanded, source_file};

use crate::attach_diff::AttachDiff;
use crate::gen_rc::RM;

/// Arguments for generating zsh startup commands
#[derive(Debug, Clone)]
pub struct ZshStartupArgs {
    pub flox_activate_tracelevel: u32,
    pub activate_d: PathBuf,
    pub is_in_place: bool,
    pub clean_up: Option<PathBuf>,
    pub auto_activate: bool,
    pub flox_bin: String,
    pub set_prompt: bool,
}

pub fn generate_zsh_startup_commands(
    args: &ZshStartupArgs,
    attach_diff: &AttachDiff,
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

    stmts.extend(attach_diff.generate_statements(args.is_in_place));

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

    if args.auto_activate {
        write!(writer, "{}", crate::hook::zsh_hook(&args.flox_bin))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use expect_test::expect;
    use shell_gen::ShellWithPath;

    use super::*;
    use crate::gen_rc::test_helpers::{render_normalized, test_startup_ctx};

    // NOTE: For these `expect!` tests, run unit tests with `UPDATE_EXPECT=1`
    //  to have it automatically update the expected value when the implementation
    //  changes.

    fn render(is_in_place: bool) -> String {
        let shell = ShellWithPath::Zsh(PathBuf::from("/bin/zsh"));
        let ctx = test_startup_ctx(shell, is_in_place);
        render_normalized(&ctx)
    }

    #[test]
    fn test_generate_zsh_startup_commands_subprocess() {
        let output = render(false);
        expect![[r#"
            typeset -g _flox_activate_tracelevel=3;
            typeset -g _activate_d=/interpreter/activate.d;
            export ADDED_VAR=ADDED_VALUE;
            export FLOX_ACTIVATE_START_SERVICES=false;
            export FLOX_ENV=/flox_env;
            export FLOX_ENV_CACHE=/flox_env_cache;
            export FLOX_ENV_DESCRIPTION=env_description;
            export FLOX_ENV_PROJECT=/flox_env_project;
            export QUOTED_VAR='QUOTED'\''VALUE';
            unset DELETED_VAR;
            source /interpreter/activate.d/zsh;
            if [[ -o interactive ]]; then source '/interpreter/activate.d/set-prompt.zsh'; fi;
            /nix/store/XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX-coreutils-9.10/bin/rm /path/to/rc/file;
        "#]]
        .assert_eq(&output);
    }

    #[test]
    fn test_generate_zsh_startup_commands_in_place() {
        let output = render(true);
        expect![[r#"
            typeset -g _flox_activate_tracelevel=3;
            typeset -g _activate_d=/interpreter/activate.d;
            export FLOX_PROMPT_COLOR_1=1;
            export FLOX_PROMPT_COLOR_2=2;
            export FLOX_PROMPT_ENVIRONMENTS=prompt_envs;
            export _FLOX_ACTIVE_ENVIRONMENTS=active_envs;
            export ADDED_VAR=ADDED_VALUE;
            export FLOX_ACTIVATE_START_SERVICES=false;
            export FLOX_ENV=/flox_env;
            export FLOX_ENV_CACHE=/flox_env_cache;
            export FLOX_ENV_DESCRIPTION=env_description;
            export FLOX_ENV_PROJECT=/flox_env_project;
            export QUOTED_VAR='QUOTED'\''VALUE';
            unset DELETED_VAR;
            source /interpreter/activate.d/zsh;
            if [[ -o interactive ]]; then source '/interpreter/activate.d/set-prompt.zsh'; fi;
            /nix/store/XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX-coreutils-9.10/bin/rm /path/to/rc/file;
        "#]]
        .assert_eq(&output);
    }
}
