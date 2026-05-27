use std::borrow::Cow;
use std::io::Write;
use std::path::PathBuf;

use anyhow::Result;
use flox_core::activate::context::InvocationType;
use indoc::indoc;
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
        Action::Deactivate(ctx) => {
            // Teardown happens in inverse order of activate.d/zsh:
            // (1) drop the `_flox_rehash` precmd hook, (2) restore
            // hashing setopts, (3) restore FPATH + re-run compinit.
            if ctx.restore_diff.is_outermost_deactivate() {
                // Undo the `_flox_rehash` precmd hook installed by activate.d/zsh.
                // Guard `unfunction` on the function existing so we don't emit
                // "no such hash table element" if the user removed it
                // themselves mid-session; `add-zsh-hook -d` is already silent
                // for an unregistered hook.
                stmts.push(
                    indoc! {r#"
                        if [[ -o interactive ]]; then
                            autoload -Uz add-zsh-hook;
                            add-zsh-hook -d precmd _flox_rehash;
                            if (( ${+functions[_flox_rehash]} )); then
                                unfunction _flox_rehash;
                            fi;
                        fi;"#}
                    .to_stmt(),
                );
                // Re-enable command hashing (zsh defaults).
                stmts.push("setopt hashcmds; setopt hashdirs;".to_stmt());
                // Restore the pre-activation FPATH and (if the user had
                // compinit initialized pre-flox) re-run their compinit so
                // completions match. Both `_FLOX_HOOK_SAVE_FPATH` and
                // `_FLOX_HOOK_SAVE_COMPINIT_DUMPFILE` are captured by
                // `activate.d/zsh` only when fpath actually changed; the
                // dumpfile is captured only when the user had compinit
                // initialized pre-flox.
                //
                // We honor the user's pre-flox compinit state rather than
                // running a bare `compinit`, so users with `compinit -u` /
                // `-d <custom>` / no compinit at all don't see surprising
                // behavior on deactivate.
                //
                // NOTE on cost: `compinit` rebuilds the completion dump,
                // which can be tens of ms on large `fpath` setups. If
                // deactivate latency ends up monitored, one option is to
                // cache the compinit result keyed on an `fpath` hash and
                // skip the rebuild when the hash matches the activate-time
                // capture.
                stmts.push(
                    indoc! {r#"
                        if [[ -n "${_FLOX_HOOK_SAVE_FPATH+set}" ]]; then
                            FPATH="$_FLOX_HOOK_SAVE_FPATH";
                            if [[ -n "${_FLOX_HOOK_SAVE_COMPINIT_DUMPFILE:-}" ]]; then
                                autoload -U compinit;
                                compinit -d "$_FLOX_HOOK_SAVE_COMPINIT_DUMPFILE";
                            fi;
                            unset _FLOX_HOOK_SAVE_FPATH _FLOX_HOOK_SAVE_COMPINIT_DUMPFILE;
                        fi;"#}
                    .to_stmt(),
                );
            }
            // Note that unsetting the prompt depends on `_activate_d` being set.
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
            if [[ -o interactive ]]; then
                autoload -Uz add-zsh-hook;
                add-zsh-hook -d precmd _flox_rehash;
                if (( ${+functions[_flox_rehash]} )); then
                    unfunction _flox_rehash;
                fi;
            fi;
            setopt hashcmds; setopt hashdirs;
            if [[ -n "${_FLOX_HOOK_SAVE_FPATH+set}" ]]; then
                FPATH="$_FLOX_HOOK_SAVE_FPATH";
                if [[ -n "${_FLOX_HOOK_SAVE_COMPINIT_DUMPFILE:-}" ]]; then
                    autoload -U compinit;
                    compinit -d "$_FLOX_HOOK_SAVE_COMPINIT_DUMPFILE";
                fi;
                unset _FLOX_HOOK_SAVE_FPATH _FLOX_HOOK_SAVE_COMPINIT_DUMPFILE;
            fi;
            if [[ -o interactive ]]; then source '/interpreter/activate.d/set-prompt.zsh'; fi;
        "#]]
        .assert_eq(&output);
    }
}
