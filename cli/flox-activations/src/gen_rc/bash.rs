use std::borrow::Cow;
use std::io::Write;
use std::path::PathBuf;

use anyhow::Result;
use shell_gen::{GenerateShell, Shell, source_file};

use crate::attach_diff::{AttachDiff, todo_drop_set_exported_unexpanded, todo_drop_unset};
use crate::gen_rc::RM;

/// Arguments for generating bash startup commands
#[derive(Debug, Clone)]
pub struct BashStartupArgs {
    pub flox_activate_tracelevel: u32,
    pub activate_d: PathBuf,
    pub flox_env: PathBuf,
    pub is_in_place: bool,
    pub clean_up: Option<PathBuf>,

    // Some(_) if it exists, None otherwise
    pub bashrc_path: Option<PathBuf>,
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
pub fn generate_bash_startup_commands(
    args: &BashStartupArgs,
    attach_diff: &AttachDiff,
    writer: &mut impl Write,
) -> Result<()> {
    let mut stmts = vec![];

    // Enable trace mode if requested
    if args.flox_activate_tracelevel >= 2 {
        stmts.push("set -x".to_stmt());
    }

    // The bashrc-sourcing dance must come before `attach_diff.generate_statements`
    // so a `flox activate` inside .bashrc can't override values
    let should_source = args.bashrc_path.is_some() && !args.is_in_place && !args.flox_sourcing_rc;
    if should_source {
        stmts.push(todo_drop_set_exported_unexpanded(
            "_flox_sourcing_rc",
            "true",
        ));
        stmts.push(source_file(args.bashrc_path.as_ref().unwrap()));
        stmts.push(todo_drop_unset("_flox_sourcing_rc"));
    }

    stmts.extend(attach_diff.generate_statements(args.is_in_place));

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

    // Set the prompt if we're in an interactive shell.
    if args.set_prompt {
        let set_prompt_path = args.activate_d.join("set-prompt.bash");
        stmts.push(
            format!(
                "if [ -t 1 ]; then source '{}'; fi;",
                set_prompt_path.display()
            )
            .to_stmt(),
        );
    }

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
        let path_str = path.to_string_lossy();
        let escaped_path = shell_escape::escape(Cow::Borrowed(path_str.as_ref()));
        stmts.push(format!("{RM} {};", escaped_path).to_stmt());
    }

    for stmt in stmts {
        stmt.generate_with_newline(Shell::Bash, writer)?;
    }

    if args.auto_activate {
        write!(writer, "{}", crate::hook::bash_hook(&args.flox_bin))?;
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
        let shell = ShellWithPath::Bash(PathBuf::from("/bin/bash"));
        let ctx = test_startup_ctx(shell, is_in_place);
        render_normalized(&ctx)
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
            export QUOTED_VAR='QUOTED'\''VALUE';
            unset DELETED_VAR;
            export _activate_d=/interpreter/activate.d;
            export _flox_activations=/flox_activations;
            export _flox_activate_tracer=TRACER;
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
            export ADDED_VAR=ADDED_VALUE;
            export FLOX_ACTIVATE_START_SERVICES=false;
            export FLOX_ENV=/flox_env;
            export FLOX_ENV_CACHE=/flox_env_cache;
            export FLOX_ENV_DESCRIPTION=env_description;
            export FLOX_ENV_PROJECT=/flox_env_project;
            export QUOTED_VAR='QUOTED'\''VALUE';
            unset DELETED_VAR;
            export _activate_d=/interpreter/activate.d;
            export _flox_activations=/flox_activations;
            export _flox_activate_tracer=TRACER;
            if [ -t 1 ]; then source '/interpreter/activate.d/set-prompt.bash'; fi;
            eval "$('/flox_activations' set-env-dirs --shell bash --flox-env "/flox_env" --env-dirs "${FLOX_ENV_DIRS:-}")";
            eval "$('/flox_activations' fix-paths --shell bash --env-dirs "$FLOX_ENV_DIRS" --path "$PATH" --manpath "${MANPATH:-}")";
            eval "$('/flox_activations' profile-scripts --shell bash --already-sourced-env-dirs "${_FLOX_SOURCED_PROFILE_SCRIPTS:-}" --env-dirs "${FLOX_ENV_DIRS:-}")";
            set +h
            set +x
            /nix/store/XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX-coreutils-9.10/bin/rm /path/to/rc/file;
        "#]].assert_eq(&output);
    }
}
