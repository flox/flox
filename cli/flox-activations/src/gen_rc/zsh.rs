use std::borrow::Cow;
use std::io::Write;
use std::path::PathBuf;

use anyhow::Result;
use flox_core::activate::context::InvocationType;
use shell_gen::{GenerateShell, Shell, set_unexported_unexpanded, source_file};

use crate::gen_rc::{Action, RM};

/// Arguments for generating zsh startup commands
#[derive(Debug, Clone)]
pub struct ZshStartupArgs {
    pub flox_activate_tracelevel: u32,
    pub activate_d: PathBuf,
    pub invocation_type: InvocationType,
    pub clean_up: Option<PathBuf>,
    pub auto_activate: bool,
    pub flox_bin: String,
    pub set_prompt: bool,
}

pub fn generate_zsh_profile_commands(
    action: &Action<ZshStartupArgs>,
    writer: &mut impl Write,
) -> Result<()> {
    let mut stmts = vec![];

    match action {
        Action::Activate { args, .. } => {
            stmts.push(set_unexported_unexpanded(
                "_flox_activate_tracelevel",
                format!("{}", &args.flox_activate_tracelevel),
            ));
            stmts.push(set_unexported_unexpanded(
                "_activate_d",
                args.activate_d.display().to_string(),
            ));
        },
        Action::Deactivate(_) => {
            // TODO: we might not need to set these in the first place
        },
    }

    // Environment variables
    match action {
        Action::Activate { args, attach_diff } => {
            stmts.extend(attach_diff.generate_statements(args.invocation_type.is_in_place()));
        },
        Action::Deactivate(ctx) => {
            stmts.extend(ctx.restore_diff.generate_deactivation_statements());
        },
    }

    // Source the zsh activate.d entry point
    match action {
        Action::Activate { args, .. } => {
            stmts.push(source_file(args.activate_d.join("zsh")));
        },
        Action::Deactivate(_) => {
            // TODO: undo everything in activate_d/zsh
            // Although note that unsetting the prompt depends on these being
            // set
        },
    }

    // Source set-prompt.zsh if we're in an interactive shell
    // set-prompt.zsh handles both setting and unsetting
    // Note for deactivate this must come after reverting environment
    // variables (which includes FLOX_PROMPT_ENVIRONMENTS)
    let set_prompt_path = match action {
        Action::Activate { args, .. } => args
            .set_prompt
            .then(|| args.activate_d.join("set-prompt.zsh")),
        Action::Deactivate(ctx) => Some(ctx.activate_d.join("set-prompt.zsh")),
    };
    if let Some(set_prompt_path) = set_prompt_path {
        // We could consult set_prompt, but hypothetically that config value
        // could change between activation and deactivation, and sourcing
        // set-prompt won't hurt
        stmts.push(
            format!(
                "if [[ -o interactive ]]; then source '{}'; fi;",
                set_prompt_path.display()
            )
            .to_stmt(),
        );
    };

    // Self-destruct rc file
    match action {
        Action::Activate { args, .. } => {
            if let Some(path) = args.clean_up.as_ref() {
                let path_str = path.to_string_lossy();
                let escaped_path = shell_escape::escape(Cow::Borrowed(path_str.as_ref()));
                stmts.push(format!("{RM} {};", escaped_path).to_stmt());
            }
        },
        Action::Deactivate(_) => {
            // No-op: deactivate has no rc file to remove.
        },
    }

    // N.B. the output of these scripts may be eval'd with backticks which have
    // the effect of removing newlines from the output, so we must ensure that
    // the output is a valid shell script fragment when represented on a single line.
    for stmt in stmts {
        stmt.generate_with_newline(Shell::Zsh, writer)?;
    }

    // Auto-activate hook registration
    match action {
        Action::Activate { args, .. } => {
            if args.auto_activate
                && matches!(
                    args.invocation_type,
                    InvocationType::Interactive | InvocationType::InPlace
                )
            {
                write!(writer, "{}", crate::hook::zsh_hook(&args.flox_bin))?;
            }
        },
        Action::Deactivate(_) => {
            // TODO: unregister the auto-activate hook
        },
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use expect_test::expect;
    use shell_gen::ShellWithPath;

    use super::*;
    use crate::gen_rc::test_helpers::{
        render_normalized,
        strip_volatile_deactivate,
        test_deactivate_ctx,
        test_startup_ctx,
    };

    // NOTE: For these `expect!` tests, run unit tests with `UPDATE_EXPECT=1`
    //  to have it automatically update the expected value when the implementation
    //  changes.

    fn render(is_in_place: bool) -> String {
        let shell = ShellWithPath::Zsh(PathBuf::from("/bin/zsh"));
        let ctx = test_startup_ctx(shell, is_in_place);
        render_normalized(&ctx)
    }

    fn render_deactivate() -> String {
        let shell = ShellWithPath::Zsh(PathBuf::from("/bin/zsh"));
        let action = Action::<ZshStartupArgs>::Deactivate(test_deactivate_ctx(shell, true));
        let mut buf = Vec::new();
        generate_zsh_profile_commands(&action, &mut buf).expect("generator should succeed");
        let output = String::from_utf8(buf).expect("output should be utf-8");
        strip_volatile_deactivate(&output)
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
            export MODIFIED_VAR=MODIFIED_VALUE;
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
            export MODIFIED_VAR=MODIFIED_VALUE;
            export QUOTED_VAR='QUOTED'\''VALUE';
            unset DELETED_VAR;
            source /interpreter/activate.d/zsh;
            if [[ -o interactive ]]; then source '/interpreter/activate.d/set-prompt.zsh'; fi;
            /nix/store/XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX-coreutils-9.10/bin/rm /path/to/rc/file;
        "#]]
        .assert_eq(&output);
    }

    #[test]
    fn generate_zsh_profile_commands_deactivate() {
        let output = render_deactivate();
        expect![[r#"
            unset ADDED_VAR;
            unset FLOX_ACTIVATE_START_SERVICES;
            unset FLOX_ENV;
            unset FLOX_ENV_CACHE;
            unset FLOX_ENV_DESCRIPTION;
            unset FLOX_ENV_DIRS;
            unset FLOX_ENV_PROJECT;
            unset FLOX_PROMPT_COLOR_1;
            unset FLOX_PROMPT_COLOR_2;
            unset FLOX_PROMPT_ENVIRONMENTS;
            unset MANPATH;
            unset PATH;
            unset QUOTED_VAR;
            unset _FLOX_ACTIVE_ENVIRONMENTS;
            export MODIFIED_VAR=MODIFIED_ORIGINAL;
            export DELETED_VAR=DELETED_ORIGINAL;
            unset _FLOX_HOOK_DIFF;
            if [[ -o interactive ]]; then source '/interpreter/activate.d/set-prompt.zsh'; fi;
        "#]]
        .assert_eq(&output);
    }
}
