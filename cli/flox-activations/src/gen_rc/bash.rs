use std::borrow::Cow;
use std::io::Write;
use std::path::PathBuf;

use anyhow::Result;
use flox_core::activate::context::InvocationType;
use flox_core::activate::vars::FLOX_ACTIVATIONS_BIN;
use shell_gen::{GenerateShell, Shell, source_file};

use crate::attach_diff::{todo_drop_set_exported_unexpanded, todo_drop_unset};
use crate::gen_rc::{Action, RM};

/// Arguments for generating bash startup commands
#[derive(Debug, Clone)]
pub struct BashStartupArgs {
    pub flox_activate_tracelevel: u32,
    pub activate_d: PathBuf,
    pub flox_env: PathBuf,
    pub invocation_type: InvocationType,
    pub clean_up: Option<PathBuf>,

    // Some(_) if it exists, None otherwise
    pub bashrc_path: Option<PathBuf>,
    pub flox_activate_tracer: String,
    pub flox_sourcing_rc: bool,
    pub flox_activations: PathBuf,
    pub register_hook: bool,
    pub flox_bin: String,
    pub set_prompt: bool,
}

// N.B. the output of these scripts may be eval'd with backticks which have
// the effect of removing newlines from the output, so we must ensure that
// the output is a valid shell script fragment when represented on a single line.
pub fn generate_bash_profile_commands(
    action: &Action<BashStartupArgs>,
    writer: &mut impl Write,
) -> Result<()> {
    let mut stmts = vec![];

    // Enable trace mode if requested
    match action {
        Action::Activate { args, .. } => {
            if args.flox_activate_tracelevel >= 2 {
                stmts.push("set -x".to_stmt());
            }
        },
        Action::Deactivate(ctx) => {
            if ctx.flox_activate_tracelevel >= 2 {
                stmts.push("set -x".to_stmt());
            }
        },
    }

    // The bashrc-sourcing dance must come before `attach_diff.generate_statements`
    // so a `flox activate` inside .bashrc can't override values
    match action {
        Action::Activate { args, .. } => {
            let should_source = args.bashrc_path.is_some()
                && !args.invocation_type.is_in_place()
                && !args.flox_sourcing_rc;
            if should_source {
                stmts.push(todo_drop_set_exported_unexpanded(
                    "_flox_sourcing_rc",
                    "true",
                ));
                stmts.push(source_file(args.bashrc_path.as_ref().unwrap()));
                stmts.push(todo_drop_unset("_flox_sourcing_rc"));
            }
        },
        Action::Deactivate(_) => {
            // No-op: no way to undo bashrc sourcing
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

    match action {
        Action::Activate { args, .. } => {
            stmts.push(todo_drop_set_exported_unexpanded(
                "_activate_d",
                args.activate_d.display().to_string(),
            ));
            // `_flox_activations` is now folded into the activation diff via
            // `single_set_envs`, so it is set on the exec'd env / emitted as a
            // single_set and unset on deactivate. Do not re-export it here.
            stmts.push(todo_drop_set_exported_unexpanded(
                "_flox_activate_tracer",
                &args.flox_activate_tracer,
            ));
        },
        Action::Deactivate(_) => {
            // No-op here — these are unset further down (after
            // set-prompt and profile.deactivate, both of which still
            // read `_activate_d` and `_flox_activate_tracer`).
        },
    }

    // Export _FLOX_INVOCATION_TYPE so it is visible to std::env::vars() when
    // computing the activation diff for stacked in-place activations. The diff
    // then handles cleanup (unset on outermost deactivate, restore outer value
    // on nested deactivate) without requiring an explicit unset here.
    match action {
        Action::Activate { args, .. } => {
            stmts.push(todo_drop_set_exported_unexpanded(
                "_FLOX_INVOCATION_TYPE",
                format!("{}", args.invocation_type),
            ));
        },
        Action::Deactivate(_) => {
            // Handled by the activation diff (added → unset, modified → restore).
        },
    }

    // Source set-prompt.bash if we're in an interactive shell
    // set-prompt.bash handles both setting and unsetting
    // Note for deactivate this must come after reverting environment
    // variables (which includes FLOX_PROMPT_ENVIRONMENTS)
    let set_prompt_path = match action {
        Action::Activate { args, .. } => args
            .set_prompt
            .then(|| args.activate_d.join("set-prompt.bash")),
        Action::Deactivate(ctx) => Some(ctx.activate_d.join("set-prompt.bash")),
    };
    if let Some(set_prompt_path) = set_prompt_path {
        // We could consult set_prompt, but hypothetically that config value
        // could change between activation and deactivation, and sourcing
        // set-prompt won't hurt
        stmts.push(
            format!(
                "if [ -t 1 ]; then source '{}'; fi;",
                set_prompt_path.display()
            )
            .to_stmt(),
        );
    };

    // We already customized the PATH and MANPATH, but the user and system
    // dotfiles may have changed them, so finish by doing this again.
    // Use generation time _FLOX_ENV because we want to guarantee we activate the
    // environment we think we're activating. Use runtime FLOX_ENV_DIRS to allow
    // RC files to perform activations.
    match action {
        Action::Activate { args, .. } => {
            stmts.push(format!(
                r#"eval "$('{}' set-env-dirs --shell {} --flox-env "{}" --env-dirs "${{FLOX_ENV_DIRS:-}}")";"#,
                args.flox_activations.display(),
                Shell::Bash,
                args.flox_env.display()
            ).to_stmt());
            stmts.push(format!(
                r#"eval "$('{}' fix-paths --shell {} --env-dirs "$FLOX_ENV_DIRS" --path "$PATH" --manpath "${{MANPATH:-}}")";"#,
                args.flox_activations.display(),
                Shell::Bash,
            ).to_stmt());
        },
        Action::Deactivate(_) => {
            // No-op: covered by environment restoration above
        },
    }

    match action {
        Action::Activate { args, .. } => {
            stmts.push(format!(
                r#"eval "$('{}' profile-scripts --shell {} --already-sourced-env-dirs "${{_FLOX_SOURCED_PROFILE_SCRIPTS:-}}" --env-dirs "${{FLOX_ENV_DIRS:-}}")";"#,
                args.flox_activations.display(),
                Shell::Bash,
            ).to_stmt());
        },
        Action::Deactivate(ctx) => {
            // Source the user's profile.deactivate.{common,bash} scripts
            // for the env being torn down, and remove it from
            // _FLOX_SOURCED_PROFILE_SCRIPTS so stacked activations stay
            // consistent. We bake in the env path at generation time
            // because by now `restore_diff` has either unset `FLOX_ENV`
            // (outermost deactivate) or restored it to the outer env's
            // value (nested), so runtime `$FLOX_ENV` is not a reliable
            // handle on the env we're tearing down.
            stmts.push(
                format!(
                    r#"eval "$('{}' profile-scripts-deactivate --shell {} --env '{}' --already-sourced-env-dirs "${{_FLOX_SOURCED_PROFILE_SCRIPTS:-}}")";"#,
                    FLOX_ACTIVATIONS_BIN.display(),
                    Shell::Bash,
                    ctx.flox_env.display()
                )
                .to_stmt(),
            );
        },
    }

    // Unset the helpers exported above. Delayed until after set-prompt
    // and profile.deactivate, both of which read `_activate_d` and
    // `_flox_activate_tracer`.
    // `_flox_activations` is unset by the activation diff (it is folded
    // into `single_set_envs`), so it is not listed here.
    match action {
        Action::Activate { .. } => {},
        Action::Deactivate(_) => {
            stmts.push("unset _activate_d _flox_activate_tracer;".to_stmt());
        },
    }

    // Disable command hashing to allow for newly installed flox packages
    // to be found immediately. We do this as the very last thing because
    // python venv activations can otherwise return nonzero return codes
    // when attempting to invoke `hash -r`.
    match action {
        Action::Activate { .. } => {
            stmts.push("set +h".to_stmt());
        },
        Action::Deactivate(ctx) => {
            // Re-enable command hashing (bash default), but only if no
            // other flox environments remain active — the outer env
            // still wants hashing off.
            if ctx.restore_diff.is_outermost_deactivate() {
                stmts.push("set -h;".to_stmt());
            }
        },
    }

    // Disable trace mode if it was enabled
    match action {
        Action::Activate { args, .. } => {
            if args.flox_activate_tracelevel >= 2 {
                stmts.push("set +x".to_stmt());
            }
        },
        Action::Deactivate(ctx) => {
            if ctx.flox_activate_tracelevel >= 2 {
                stmts.push("set +x".to_stmt());
            }
        },
    }

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

    for stmt in stmts {
        stmt.generate_with_newline(Shell::Bash, writer)?;
    }

    // Auto-activate hook registration
    match action {
        Action::Activate { args, .. } => {
            if args.register_hook
                && matches!(
                    args.invocation_type,
                    InvocationType::Interactive | InvocationType::InPlace
                )
            {
                write!(writer, "{}", crate::hook::bash_hook(&args.flox_bin))?;
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
        let shell = ShellWithPath::Bash(PathBuf::from("/bin/bash"));
        let ctx = test_startup_ctx(shell, is_in_place);
        render_normalized(&ctx)
    }

    fn render_deactivate(flox_activate_tracelevel: u32) -> String {
        let shell = ShellWithPath::Bash(PathBuf::from("/bin/bash"));
        let mut ctx = test_deactivate_ctx(shell, true);
        ctx.flox_activate_tracelevel = flox_activate_tracelevel;
        let action = Action::<BashStartupArgs>::Deactivate(ctx);
        let mut buf = Vec::new();
        generate_bash_profile_commands(&action, &mut buf).expect("generator should succeed");
        let output = String::from_utf8(buf).expect("output should be utf-8");
        strip_volatile_deactivate(&output)
    }

    #[test]
    fn test_generate_bash_startup_commands_subprocess() {
        let output = render(false);
        expect![[r#"
            set -x
            export _flox_sourcing_rc=true;
            source /home/user/.bashrc;
            unset _flox_sourcing_rc;
            export ADDED_VAR=ADDED_VALUE;
            export FLOX_ACTIVATE_START_SERVICES=false;
            export FLOX_ENV=/flox_env;
            export FLOX_ENV_CACHE=/flox_env_cache;
            export FLOX_ENV_DESCRIPTION=env_description;
            export FLOX_ENV_PROJECT=/flox_env_project;
            export MODIFIED_VAR=MODIFIED_VALUE;
            export QUOTED_VAR='QUOTED'\''VALUE';
            unset DELETED_VAR;
            export _activate_d=/interpreter/activate.d;
            export _flox_activate_tracer=TRACER;
            export _FLOX_INVOCATION_TYPE=interactive;
            if [ -t 1 ]; then source '/interpreter/activate.d/set-prompt.bash'; fi;
            eval "$('/flox_activations' set-env-dirs --shell bash --flox-env "/flox_env" --env-dirs "${FLOX_ENV_DIRS:-}")";
            eval "$('/flox_activations' fix-paths --shell bash --env-dirs "$FLOX_ENV_DIRS" --path "$PATH" --manpath "${MANPATH:-}")";
            eval "$('/flox_activations' profile-scripts --shell bash --already-sourced-env-dirs "${_FLOX_SOURCED_PROFILE_SCRIPTS:-}" --env-dirs "${FLOX_ENV_DIRS:-}")";
            set +h
            set +x
            /nix/store/XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX-coreutils-9.10/bin/rm /path/to/rc/file;
        "#]].assert_eq(&output);
    }

    #[test]
    fn test_generate_bash_startup_commands_in_place() {
        let output = render(true);
        expect![[r#"
            set -x
            export FLOX_PROMPT_COLOR_1=1;
            export FLOX_PROMPT_COLOR_2=2;
            export FLOX_PROMPT_ENVIRONMENTS=prompt_envs;
            export _FLOX_ACTIVE_ENVIRONMENTS=active_envs;
            export _flox_activations=/flox_activations;
            export ADDED_VAR=ADDED_VALUE;
            export FLOX_ACTIVATE_START_SERVICES=false;
            export FLOX_ENV=/flox_env;
            export FLOX_ENV_CACHE=/flox_env_cache;
            export FLOX_ENV_DESCRIPTION=env_description;
            export FLOX_ENV_PROJECT=/flox_env_project;
            export MODIFIED_VAR=MODIFIED_VALUE;
            export QUOTED_VAR='QUOTED'\''VALUE';
            unset DELETED_VAR;
            export _activate_d=/interpreter/activate.d;
            export _flox_activate_tracer=TRACER;
            export _FLOX_INVOCATION_TYPE=inplace;
            if [ -t 1 ]; then source '/interpreter/activate.d/set-prompt.bash'; fi;
            eval "$('/flox_activations' set-env-dirs --shell bash --flox-env "/flox_env" --env-dirs "${FLOX_ENV_DIRS:-}")";
            eval "$('/flox_activations' fix-paths --shell bash --env-dirs "$FLOX_ENV_DIRS" --path "$PATH" --manpath "${MANPATH:-}")";
            eval "$('/flox_activations' profile-scripts --shell bash --already-sourced-env-dirs "${_FLOX_SOURCED_PROFILE_SCRIPTS:-}" --env-dirs "${FLOX_ENV_DIRS:-}")";
            set +h
            set +x
            /nix/store/XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX-coreutils-9.10/bin/rm /path/to/rc/file;
        "#]].assert_eq(&output);
    }

    #[test]
    fn generate_bash_profile_deactivate() {
        let output = render_deactivate(0);
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
            unset _FLOX_HOOK_DIFF;
            unset _FLOX_INVOCATION_TYPE;
            unset _flox_activations;
            export MODIFIED_VAR=MODIFIED_ORIGINAL;
            export DELETED_VAR=DELETED_ORIGINAL;
            if [ -t 1 ]; then source '/interpreter/activate.d/set-prompt.bash'; fi;
            eval "$('/flox_activations' profile-scripts-deactivate --shell bash --env '/flox_env' --already-sourced-env-dirs "${_FLOX_SOURCED_PROFILE_SCRIPTS:-}")";
            unset _activate_d _flox_activate_tracer;
            set -h;
        "#]]
        .assert_eq(&output);
    }

    #[test]
    fn generate_bash_profile_deactivate_traced() {
        // The traced variant is the untraced body wrapped in
        // `set -x` / `set +x`. The body itself is snapshotted by
        // `generate_bash_profile_deactivate`.
        let traced = render_deactivate(2);
        let untraced = render_deactivate(0);
        assert_eq!(traced, format!("set -x\n{untraced}set +x\n"));
    }
}
