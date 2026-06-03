use std::borrow::Cow;
use std::io::Write;
use std::path::PathBuf;

use anyhow::Result;
use flox_core::activate::context::InvocationType;
use flox_core::activate::vars::FLOX_ACTIVATIONS_BIN;
use shell_gen::{GenerateShell, Shell, set_unexported_unexpanded};

use crate::attach_diff::todo_drop_set_exported_unexpanded;
use crate::gen_rc::{Action, RM};

/// Arguments for generating tcsh startup commands
#[derive(Debug, Clone)]
pub struct TcshStartupArgs {
    pub flox_activate_tracelevel: u32,
    pub activate_d: PathBuf,
    pub flox_env: PathBuf,
    pub invocation_type: InvocationType,
    pub clean_up: Option<PathBuf>,

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
pub fn generate_tcsh_profile_commands(
    action: &Action<TcshStartupArgs>,
    writer: &mut impl Write,
) -> Result<()> {
    let mut stmts = vec![];

    // Enable trace mode if requested
    match action {
        Action::Activate { args, .. } => {
            if args.flox_activate_tracelevel >= 2 {
                stmts.push("set verbose".to_stmt());
            }
        },
        Action::Deactivate(_) => {
            // TODO: emit `set verbose` when tracelevel >= 2
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
            // TODO: we shouldn't be exporting these in the first place
            // Although note that unsetting the prompt depends on these being
            // set
        },
    }

    // Emit _FLOX_INVOCATION_TYPE as a shell-local (non-exported) variable so
    // that `flox deactivate --print-script` can distinguish interactive
    // subshells from in-place activations without reading state.json.
    match action {
        Action::Activate { args, .. } => {
            stmts.push(set_unexported_unexpanded(
                "_FLOX_INVOCATION_TYPE",
                format!("{}", args.invocation_type),
            ));
        },
        Action::Deactivate(_) => {
            // tcsh uses `set` (not setenv) for shell-local vars, so the
            // corresponding teardown is `unset` (not `unsetenv`).
            stmts.push("unset _FLOX_INVOCATION_TYPE;".to_stmt());
        },
    }

    // Source set-prompt.tcsh if we're in an interactive shell
    // set-prompt.tcsh handles both setting and unsetting
    // Note for deactivate this must come after reverting environment
    // variables (which includes FLOX_PROMPT_ENVIRONMENTS)
    let set_prompt_path = match action {
        Action::Activate { args, .. } => args
            .set_prompt
            .then(|| args.activate_d.join("set-prompt.tcsh")),
        Action::Deactivate(ctx) => Some(ctx.activate_d.join("set-prompt.tcsh")),
    };
    if let Some(set_prompt_path) = set_prompt_path {
        // We could consult set_prompt, but hypothetically that config value
        // could change between activation and deactivation, and sourcing
        // set-prompt won't hurt
        stmts.push(
            format!(
                "if ( $?tty ) then; source '{}'; endif;",
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
            stmts.push(r#"if (! $?FLOX_ENV_DIRS) setenv FLOX_ENV_DIRS "empty";"#.to_stmt());

            stmts.push(format!(
                r#"eval "`'{}' set-env-dirs --shell {} --flox-env '{}' --env-dirs $FLOX_ENV_DIRS:q`";"#,
                args.flox_activations.display(),
                Shell::Tcsh,
                args.flox_env.display(),
            ).to_stmt());

            stmts.push(r#"if (! $?MANPATH) setenv MANPATH "empty";"#.to_stmt());

            stmts.push(format!(
                r#"eval "`'{}' fix-paths --shell {} --env-dirs $FLOX_ENV_DIRS:q --path $PATH:q --manpath $MANPATH:q`";"#,
                args.flox_activations.display(),
                Shell::Tcsh,
            ).to_stmt());
        },
        Action::Deactivate(_) => {
            // No-op: covered by environment restoration above
        },
    }

    // Modern versions of tcsh support the ":Q" modifier for passing empty args
    // on the command line, but versions prior to 6.23 do not have a way to do
    // that, so to support these versions we will instead avoid passing the
    // --already-sourced-env-dirs argument altogether when there is no default
    // value to be passed.
    match action {
        Action::Activate { args, .. } => {
            stmts.push("set _already_sourced_args = ();".to_stmt());

            stmts.push(
                r#"if ($?_FLOX_SOURCED_PROFILE_SCRIPTS) set _already_sourced_args = ( --already-sourced-env-dirs `echo $_FLOX_SOURCED_PROFILE_SCRIPTS:q` );"#.to_stmt()
            );

            stmts.push(format!(
                r#"eval "`'{}' profile-scripts --shell {} --env-dirs $FLOX_ENV_DIRS:q $_already_sourced_args:q`";"#,
                args.flox_activations.display(),
                Shell::Tcsh,
            ).to_stmt());
        },
        Action::Deactivate(ctx) => {
            // Source the user's profile.deactivate.{common,tcsh} scripts
            // for the env being torn down, and remove it from
            // _FLOX_SOURCED_PROFILE_SCRIPTS so stacked activations stay
            // consistent. We bake in the env path at generation time —
            // using runtime `$FLOX_ENV:q` here would be fatal in tcsh
            // once `restore_diff` has unset FLOX_ENV (referencing an
            // undefined variable aborts the script). `_already_sourced_args`
            // is reused from the activate branch above so older tcsh
            // (pre-6.23) can omit `--already-sourced-env-dirs` entirely
            // when the var is unset.
            stmts.push("set _already_sourced_args = ();".to_stmt());

            stmts.push(
                r#"if ($?_FLOX_SOURCED_PROFILE_SCRIPTS) set _already_sourced_args = ( --already-sourced-env-dirs `echo $_FLOX_SOURCED_PROFILE_SCRIPTS:q` );"#.to_stmt()
            );

            stmts.push(
                format!(
                    r#"eval "`'{}' profile-scripts-deactivate --shell {} --env '{}' $_already_sourced_args:q`";"#,
                    FLOX_ACTIVATIONS_BIN.display(),
                    Shell::Tcsh,
                    ctx.flox_env.display()
                )
                .to_stmt(),
            );
        },
    }

    // Disable command hashing to allow for newly installed flox packages
    // to be found immediately. We do this as the very last thing because
    // python venv activations can otherwise return nonzero return codes
    // when attempting to invoke `hash -r`.
    match action {
        Action::Activate { .. } => {
            stmts.push("unhash;".to_stmt());
        },
        Action::Deactivate(ctx) => {
            // Re-enable command hashing by rebuilding the hash table,
            // but only if no other flox environments remain active —
            // the outer env still wants hashing off.
            if ctx.restore_diff.is_outermost_deactivate() {
                stmts.push("rehash;".to_stmt());
            }
        },
    }

    // Disable trace mode if it was enabled
    match action {
        Action::Activate { args, .. } => {
            if args.flox_activate_tracelevel >= 2 {
                stmts.push("unset verbose;".to_stmt());
            }
        },
        Action::Deactivate(_) => {
            // TODO: unset verbose
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
        stmt.generate_with_newline(Shell::Tcsh, writer)?;
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
                write!(writer, "{}", crate::hook::tcsh_hook(&args.flox_bin))?;
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
        let shell = ShellWithPath::Tcsh(PathBuf::from("/bin/tcsh"));
        let ctx = test_startup_ctx(shell, is_in_place);
        render_normalized(&ctx)
    }

    fn render_deactivate() -> String {
        let shell = ShellWithPath::Tcsh(PathBuf::from("/bin/tcsh"));
        let action = Action::<TcshStartupArgs>::Deactivate(test_deactivate_ctx(shell, true));
        let mut buf = Vec::new();
        generate_tcsh_profile_commands(&action, &mut buf).expect("generator should succeed");
        let output = String::from_utf8(buf).expect("output should be utf-8");
        strip_volatile_deactivate(&output)
    }

    #[test]
    fn test_generate_tcsh_startup_commands_subprocess() {
        let output = render(false);
        expect![[r#"
            set verbose
            setenv ADDED_VAR ADDED_VALUE;
            setenv FLOX_ACTIVATE_START_SERVICES false;
            setenv FLOX_ENV /flox_env;
            setenv FLOX_ENV_CACHE /flox_env_cache;
            setenv FLOX_ENV_DESCRIPTION env_description;
            setenv FLOX_ENV_PROJECT /flox_env_project;
            setenv MODIFIED_VAR MODIFIED_VALUE;
            setenv QUOTED_VAR 'QUOTED'\''VALUE';
            unsetenv DELETED_VAR;
            setenv _activate_d /interpreter/activate.d;
            setenv _flox_activate_tracer TRACER;
            set _FLOX_INVOCATION_TYPE = interactive;
            if ( $?tty ) then; source '/interpreter/activate.d/set-prompt.tcsh'; endif;
            if (! $?FLOX_ENV_DIRS) setenv FLOX_ENV_DIRS "empty";
            eval "`'/flox_activations' set-env-dirs --shell tcsh --flox-env '/flox_env' --env-dirs $FLOX_ENV_DIRS:q`";
            if (! $?MANPATH) setenv MANPATH "empty";
            eval "`'/flox_activations' fix-paths --shell tcsh --env-dirs $FLOX_ENV_DIRS:q --path $PATH:q --manpath $MANPATH:q`";
            set _already_sourced_args = ();
            if ($?_FLOX_SOURCED_PROFILE_SCRIPTS) set _already_sourced_args = ( --already-sourced-env-dirs `echo $_FLOX_SOURCED_PROFILE_SCRIPTS:q` );
            eval "`'/flox_activations' profile-scripts --shell tcsh --env-dirs $FLOX_ENV_DIRS:q $_already_sourced_args:q`";
            unhash;
            unset verbose;
            /nix/store/XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX-coreutils-9.10/bin/rm /path/to/rc/file;
        "#]].assert_eq(&output);
    }

    #[test]
    fn test_generate_tcsh_startup_commands_in_place() {
        let output = render(true);
        expect![[r#"
            set verbose
            setenv FLOX_PROMPT_COLOR_1 1;
            setenv FLOX_PROMPT_COLOR_2 2;
            setenv FLOX_PROMPT_ENVIRONMENTS prompt_envs;
            setenv _FLOX_ACTIVE_ENVIRONMENTS active_envs;
            setenv _flox_activations /flox_activations;
            setenv ADDED_VAR ADDED_VALUE;
            setenv FLOX_ACTIVATE_START_SERVICES false;
            setenv FLOX_ENV /flox_env;
            setenv FLOX_ENV_CACHE /flox_env_cache;
            setenv FLOX_ENV_DESCRIPTION env_description;
            setenv FLOX_ENV_PROJECT /flox_env_project;
            setenv MODIFIED_VAR MODIFIED_VALUE;
            setenv QUOTED_VAR 'QUOTED'\''VALUE';
            unsetenv DELETED_VAR;
            setenv _activate_d /interpreter/activate.d;
            setenv _flox_activate_tracer TRACER;
            set _FLOX_INVOCATION_TYPE = inplace;
            if ( $?tty ) then; source '/interpreter/activate.d/set-prompt.tcsh'; endif;
            if (! $?FLOX_ENV_DIRS) setenv FLOX_ENV_DIRS "empty";
            eval "`'/flox_activations' set-env-dirs --shell tcsh --flox-env '/flox_env' --env-dirs $FLOX_ENV_DIRS:q`";
            if (! $?MANPATH) setenv MANPATH "empty";
            eval "`'/flox_activations' fix-paths --shell tcsh --env-dirs $FLOX_ENV_DIRS:q --path $PATH:q --manpath $MANPATH:q`";
            set _already_sourced_args = ();
            if ($?_FLOX_SOURCED_PROFILE_SCRIPTS) set _already_sourced_args = ( --already-sourced-env-dirs `echo $_FLOX_SOURCED_PROFILE_SCRIPTS:q` );
            eval "`'/flox_activations' profile-scripts --shell tcsh --env-dirs $FLOX_ENV_DIRS:q $_already_sourced_args:q`";
            unhash;
            unset verbose;
            /nix/store/XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX-coreutils-9.10/bin/rm /path/to/rc/file;
        "#]].assert_eq(&output);
    }

    #[test]
    fn generate_tcsh_profile_deactivate() {
        let output = render_deactivate();
        expect![[r#"
            unsetenv ADDED_VAR;
            unsetenv FLOX_ACTIVATE_START_SERVICES;
            unsetenv FLOX_ENV;
            unsetenv FLOX_ENV_CACHE;
            unsetenv FLOX_ENV_DESCRIPTION;
            unsetenv FLOX_ENV_DIRS;
            unsetenv FLOX_ENV_PROJECT;
            unsetenv FLOX_PROMPT_COLOR_1;
            unsetenv FLOX_PROMPT_COLOR_2;
            unsetenv FLOX_PROMPT_ENVIRONMENTS;
            unsetenv MANPATH;
            unsetenv PATH;
            unsetenv QUOTED_VAR;
            unsetenv _FLOX_ACTIVE_ENVIRONMENTS;
            unsetenv _flox_activations;
            setenv MODIFIED_VAR MODIFIED_ORIGINAL;
            setenv DELETED_VAR DELETED_ORIGINAL;
            unsetenv _FLOX_HOOK_DIFF;
            unset _FLOX_INVOCATION_TYPE;
            if ( $?tty ) then; source '/interpreter/activate.d/set-prompt.tcsh'; endif;
            set _already_sourced_args = ();
            if ($?_FLOX_SOURCED_PROFILE_SCRIPTS) set _already_sourced_args = ( --already-sourced-env-dirs `echo $_FLOX_SOURCED_PROFILE_SCRIPTS:q` );
            eval "`'/flox_activations' profile-scripts-deactivate --shell tcsh --env '/flox_env' $_already_sourced_args:q`";
            rehash;
        "#]]
        .assert_eq(&output);
    }
}
