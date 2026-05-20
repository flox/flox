use std::borrow::Cow;
use std::io::Write;
use std::path::PathBuf;

use anyhow::Result;
use flox_core::activate::context::InvocationType;
use shell_gen::{GenerateShell, Shell};

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
    pub auto_activate: bool,
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
        Action::Deactivate => {
            // TODO: emit `set verbose` when tracelevel >= 2
        },
    }

    // Environment variables
    match action {
        Action::Activate { args, attach_diff } => {
            stmts.extend(attach_diff.generate_statements(args.invocation_type.is_in_place()));
        },
        Action::Deactivate => {
            // TODO: decode `_FLOX_HOOK_DIFF` and emit restores.
        },
    }

    match action {
        Action::Activate { args, .. } => {
            stmts.push(todo_drop_set_exported_unexpanded(
                "_activate_d",
                args.activate_d.display().to_string(),
            ));
            stmts.push(todo_drop_set_exported_unexpanded(
                "_flox_activations",
                args.flox_activations.display().to_string(),
            ));
            stmts.push(todo_drop_set_exported_unexpanded(
                "_flox_activate_tracer",
                &args.flox_activate_tracer,
            ));
        },
        Action::Deactivate => {
            // TODO: we shouldn't be exporting these in the first place
        },
    }

    // Set the prompt if we're in an interactive shell.
    match action {
        Action::Activate { args, .. } => {
            if args.set_prompt {
                let set_prompt_path = args.activate_d.join("set-prompt.tcsh");
                stmts.push(
                    format!(
                        "if ( $?tty ) then; source '{}'; endif;",
                        set_prompt_path.display()
                    )
                    .to_stmt(),
                );
            }
        },
        Action::Deactivate => {
            // TODO: revert the prompt.
        },
    }

    // We already customized the PATH and MANPATH, but the user and system
    // dotfiles may have changed them, so finish by doing this again.
    // Use generation time _FLOX_ENV because we want to guarantee we activate the
    // environment we think we're activating. Use runtime FLOX_ENV_DIRS to allow
    // RC files to perform activations.
    match action {
        Action::Activate { args, .. } => {
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
        },
        Action::Deactivate => {
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
                r#"eval "`'{}' profile-scripts --shell tcsh --env-dirs $FLOX_ENV_DIRS:q $_already_sourced_args:q`";"#,
                args.flox_activations.display()
            ).to_stmt());
        },
        Action::Deactivate => {
            // TODO: run profile.deactivate.tcsh
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
        Action::Deactivate => {
            // TODO: decide whether to restore prior hashing state.
        },
    }

    // Disable trace mode if it was enabled
    match action {
        Action::Activate { args, .. } => {
            if args.flox_activate_tracelevel >= 2 {
                stmts.push("unset verbose;".to_stmt());
            }
        },
        Action::Deactivate => {
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
        Action::Deactivate => {
            // No-op: deactivate has no rc file to remove.
        },
    }

    for stmt in stmts {
        stmt.generate_with_newline(Shell::Tcsh, writer)?;
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
                write!(writer, "{}", crate::hook::tcsh_hook(&args.flox_bin))?;
            }
        },
        Action::Deactivate => {
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
    use crate::gen_rc::test_helpers::{render_normalized, test_startup_ctx};

    // NOTE: For these `expect!` tests, run unit tests with `UPDATE_EXPECT=1`
    //  to have it automatically update the expected value when the implementation
    //  changes.

    fn render(is_in_place: bool) -> String {
        let shell = ShellWithPath::Tcsh(PathBuf::from("/bin/tcsh"));
        let ctx = test_startup_ctx(shell, is_in_place);
        render_normalized(&ctx)
    }

    fn render_deactivate() -> String {
        let action = Action::<TcshStartupArgs>::Deactivate;
        let mut buf = Vec::new();
        generate_tcsh_profile_commands(&action, &mut buf).expect("generator should succeed");
        String::from_utf8(buf).expect("output should be utf-8")
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
            setenv QUOTED_VAR 'QUOTED'\''VALUE';
            unsetenv DELETED_VAR;
            setenv _activate_d /interpreter/activate.d;
            setenv _flox_activations /flox_activations;
            setenv _flox_activate_tracer TRACER;
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
            setenv ADDED_VAR ADDED_VALUE;
            setenv FLOX_ACTIVATE_START_SERVICES false;
            setenv FLOX_ENV /flox_env;
            setenv FLOX_ENV_CACHE /flox_env_cache;
            setenv FLOX_ENV_DESCRIPTION env_description;
            setenv FLOX_ENV_PROJECT /flox_env_project;
            setenv QUOTED_VAR 'QUOTED'\''VALUE';
            unsetenv DELETED_VAR;
            setenv _activate_d /interpreter/activate.d;
            setenv _flox_activations /flox_activations;
            setenv _flox_activate_tracer TRACER;
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
        expect![""].assert_eq(&output);
    }
}
