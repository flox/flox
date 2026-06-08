use std::borrow::Cow;
use std::io::Write;
use std::path::PathBuf;

use anyhow::Result;
use flox_core::activate::context::{AutoActivateFishMode, InvocationType};
use flox_core::activate::vars::FLOX_ACTIVATIONS_BIN;
use shell_gen::{GenerateShell, Shell};

use crate::attach_diff::todo_drop_set_exported_unexpanded;
use crate::gen_rc::{Action, RM};

/// Arguments for generating fish startup commands
#[derive(Debug, Clone)]
pub struct FishStartupArgs {
    pub flox_activate_tracelevel: u32,
    pub activate_d: PathBuf,
    pub flox_env: PathBuf,
    pub invocation_type: InvocationType,
    pub clean_up: Option<PathBuf>,

    // Some(_) if it exists, None otherwise
    pub flox_sourcing_rc: bool,
    pub flox_activate_tracer: String,
    pub flox_activations: PathBuf,
    pub register_hook: bool,
    pub flox_bin: String,
    pub auto_activate_fish_mode: Option<AutoActivateFishMode>,
    pub set_prompt: bool,
}

// N.B. the output of these scripts may be eval'd with backticks which have
// the effect of removing newlines from the output, so we must ensure that
// the output is a valid shell script fragment when represented on a single line.
pub fn generate_fish_profile_commands(
    action: &Action<FishStartupArgs>,
    writer: &mut impl Write,
) -> Result<()> {
    let mut stmts = vec![];

    // Enable trace mode if requested
    match action {
        Action::Activate { args, .. } => {
            if args.flox_activate_tracelevel >= 2 {
                stmts.push(todo_drop_set_exported_unexpanded("fish_trace", "1").to_stmt());
            }
        },
        Action::Deactivate(ctx) => {
            if ctx.flox_activate_tracelevel >= 2 {
                stmts.push(todo_drop_set_exported_unexpanded("fish_trace", "1").to_stmt());
            }
        },
    }

    // The fish --init-command option allows us to source our startup
    // file after the normal configuration has been processed, so there
    // is no requirement to go back and source the user's own config
    // as we do in bash.

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

    // Source set-prompt.fish if we're in an interactive shell
    // set-prompt.fish handles both setting and unsetting
    // Note for deactivate this must come after reverting environment
    // variables (which includes FLOX_PROMPT_ENVIRONMENTS)
    let set_prompt_path = match action {
        Action::Activate { args, .. } => args
            .set_prompt
            .then(|| args.activate_d.join("set-prompt.fish")),
        Action::Deactivate(ctx) => Some(ctx.activate_d.join("set-prompt.fish")),
    };
    if let Some(set_prompt_path) = set_prompt_path {
        // We could consult set_prompt, but hypothetically that config value
        // could change between activation and deactivation, and sourcing
        // set-prompt won't hurt
        stmts.push(format!("if isatty 1; source '{}'; end;", set_prompt_path.display()).to_stmt());
    };

    // We already customized the PATH and MANPATH, but the user and system
    // dotfiles may have changed them, so finish by doing this again.
    //
    // fish doesn't have {foo:-} syntax, so we need to provide a temporary variable
    // (foo_with_default) that is either the runtime (not generation-time) value
    // or the string 'empty'.
    match action {
        Action::Activate { args, .. } => {
            stmts.push(
                r#"set -gx FLOX_ENV_DIRS (if set -q FLOX_ENV_DIRS; echo "$FLOX_ENV_DIRS"; else; echo empty; end);"#.to_stmt()
            );

            stmts.push(
                format!(
                    r#"{} set-env-dirs --shell {} --flox-env "{}" --env-dirs "$FLOX_ENV_DIRS" | source;"#,
                    args.flox_activations.display(),
                    Shell::Fish,
                    args.flox_env.display()
                )
                .to_stmt(),
            );

            stmts.push(
                r#"set -gx MANPATH (if set -q MANPATH; echo "$MANPATH"; else; echo empty; end);"#
                    .to_stmt(),
            );

            stmts.push(format!(
                r#"{} fix-paths --shell {} --env-dirs "$FLOX_ENV_DIRS" --path "$PATH" --manpath "$MANPATH" | source;"#,
                args.flox_activations.display(),
                Shell::Fish,
            ).to_stmt());
        },
        Action::Deactivate(_) => {
            // No-op: covered by environment restoration above
        },
    }

    match action {
        Action::Activate { args, .. } => {
            stmts.push(
                r#"set -g  _FLOX_SOURCED_PROFILE_SCRIPTS (if set -q _FLOX_SOURCED_PROFILE_SCRIPTS; echo "$_FLOX_SOURCED_PROFILE_SCRIPTS"; else; echo ""; end);"#.to_string()
            .to_stmt());

            stmts.push(format!(
                r#"{} profile-scripts --shell {} --already-sourced-env-dirs  "$_FLOX_SOURCED_PROFILE_SCRIPTS" --env-dirs "$FLOX_ENV_DIRS" | source;"#,
                args.flox_activations.display(),
                Shell::Fish,
            ).to_stmt());
        },
        Action::Deactivate(ctx) => {
            // Source the user's profile.deactivate.{common,fish} scripts
            // for the env being torn down, and remove it from
            // _FLOX_SOURCED_PROFILE_SCRIPTS so stacked activations stay
            // consistent. We bake in the env path at generation time
            // because by now `restore_diff` has either unset `FLOX_ENV`
            // (outermost deactivate) or restored it to the outer env's
            // value (nested), so runtime `$FLOX_ENV` is not a reliable
            // handle on the env we're tearing down. The fallback below
            // mirrors the activate-side initializer for unset vars.
            stmts.push(
                format!(
                    r#"{} profile-scripts-deactivate --shell {} --env '{}' --already-sourced-env-dirs (if set -q _FLOX_SOURCED_PROFILE_SCRIPTS; echo "$_FLOX_SOURCED_PROFILE_SCRIPTS"; else; echo ""; end) | source;"#,
                    FLOX_ACTIVATIONS_BIN.display(),
                    Shell::Fish,
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
            stmts.push("set -e _activate_d _flox_activate_tracer;".to_stmt());
        },
    }

    // fish does not use hashing in the same way bash does, so there's
    // nothing to be done here by way of that requirement.

    // Disable trace mode if it was enabled
    match action {
        Action::Activate { args, .. } => {
            if args.flox_activate_tracelevel >= 2 {
                stmts.push("set -gx fish_trace 0;".to_stmt());
            }
        },
        Action::Deactivate(ctx) => {
            if ctx.flox_activate_tracelevel >= 2 {
                stmts.push("set -gx fish_trace 0;".to_stmt());
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
        stmt.generate_with_newline(Shell::Fish, writer)?;
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
                if let Some(mode) = &args.auto_activate_fish_mode {
                    writeln!(writer, "set -gx FLOX_AUTO_ACTIVATE_FISH_MODE {mode};")?;
                }
                write!(writer, "{}", crate::hook::fish_hook(&args.flox_bin))?;
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
        let shell = ShellWithPath::Fish(PathBuf::from("/fish"));
        let ctx = test_startup_ctx(shell, is_in_place);
        render_normalized(&ctx)
    }

    fn render_deactivate(flox_activate_tracelevel: u32) -> String {
        let shell = ShellWithPath::Fish(PathBuf::from("/fish"));
        let mut ctx = test_deactivate_ctx(shell, true);
        ctx.flox_activate_tracelevel = flox_activate_tracelevel;
        let action = Action::<FishStartupArgs>::Deactivate(ctx);
        let mut buf = Vec::new();
        generate_fish_profile_commands(&action, &mut buf).expect("generator should succeed");
        let output = String::from_utf8(buf).expect("output should be utf-8");
        strip_volatile_deactivate(&output)
    }

    #[test]
    fn test_generate_fish_startup_commands_subprocess() {
        let output = render(false);
        expect![[r#"
            set -gx fish_trace 1;
            set -gx ADDED_VAR ADDED_VALUE;
            set -gx FLOX_ACTIVATE_START_SERVICES false;
            set -gx FLOX_ENV /flox_env;
            set -gx FLOX_ENV_CACHE /flox_env_cache;
            set -gx FLOX_ENV_DESCRIPTION env_description;
            set -gx FLOX_ENV_PROJECT /flox_env_project;
            set -gx MODIFIED_VAR MODIFIED_VALUE;
            set -gx QUOTED_VAR 'QUOTED'\''VALUE';
            set -e DELETED_VAR;
            set -gx _activate_d /interpreter/activate.d;
            set -gx _flox_activate_tracer TRACER;
            set -gx _FLOX_INVOCATION_TYPE interactive;
            if isatty 1; source '/interpreter/activate.d/set-prompt.fish'; end;
            set -gx FLOX_ENV_DIRS (if set -q FLOX_ENV_DIRS; echo "$FLOX_ENV_DIRS"; else; echo empty; end);
            /flox_activations set-env-dirs --shell fish --flox-env "/flox_env" --env-dirs "$FLOX_ENV_DIRS" | source;
            set -gx MANPATH (if set -q MANPATH; echo "$MANPATH"; else; echo empty; end);
            /flox_activations fix-paths --shell fish --env-dirs "$FLOX_ENV_DIRS" --path "$PATH" --manpath "$MANPATH" | source;
            set -g  _FLOX_SOURCED_PROFILE_SCRIPTS (if set -q _FLOX_SOURCED_PROFILE_SCRIPTS; echo "$_FLOX_SOURCED_PROFILE_SCRIPTS"; else; echo ""; end);
            /flox_activations profile-scripts --shell fish --already-sourced-env-dirs  "$_FLOX_SOURCED_PROFILE_SCRIPTS" --env-dirs "$FLOX_ENV_DIRS" | source;
            set -gx fish_trace 0;
            /nix/store/XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX-coreutils-9.10/bin/rm /path/to/rc/file;
        "#]].assert_eq(&output);
    }

    #[test]
    fn test_generate_fish_startup_commands_in_place() {
        let output = render(true);
        expect![[r#"
            set -gx fish_trace 1;
            set -gx FLOX_PROMPT_COLOR_1 1;
            set -gx FLOX_PROMPT_COLOR_2 2;
            set -gx FLOX_PROMPT_ENVIRONMENTS prompt_envs;
            set -gx _FLOX_ACTIVE_ENVIRONMENTS active_envs;
            set -gx _flox_activations /flox_activations;
            set -gx ADDED_VAR ADDED_VALUE;
            set -gx FLOX_ACTIVATE_START_SERVICES false;
            set -gx FLOX_ENV /flox_env;
            set -gx FLOX_ENV_CACHE /flox_env_cache;
            set -gx FLOX_ENV_DESCRIPTION env_description;
            set -gx FLOX_ENV_PROJECT /flox_env_project;
            set -gx MODIFIED_VAR MODIFIED_VALUE;
            set -gx QUOTED_VAR 'QUOTED'\''VALUE';
            set -e DELETED_VAR;
            set -gx _activate_d /interpreter/activate.d;
            set -gx _flox_activate_tracer TRACER;
            set -gx _FLOX_INVOCATION_TYPE inplace;
            if isatty 1; source '/interpreter/activate.d/set-prompt.fish'; end;
            set -gx FLOX_ENV_DIRS (if set -q FLOX_ENV_DIRS; echo "$FLOX_ENV_DIRS"; else; echo empty; end);
            /flox_activations set-env-dirs --shell fish --flox-env "/flox_env" --env-dirs "$FLOX_ENV_DIRS" | source;
            set -gx MANPATH (if set -q MANPATH; echo "$MANPATH"; else; echo empty; end);
            /flox_activations fix-paths --shell fish --env-dirs "$FLOX_ENV_DIRS" --path "$PATH" --manpath "$MANPATH" | source;
            set -g  _FLOX_SOURCED_PROFILE_SCRIPTS (if set -q _FLOX_SOURCED_PROFILE_SCRIPTS; echo "$_FLOX_SOURCED_PROFILE_SCRIPTS"; else; echo ""; end);
            /flox_activations profile-scripts --shell fish --already-sourced-env-dirs  "$_FLOX_SOURCED_PROFILE_SCRIPTS" --env-dirs "$FLOX_ENV_DIRS" | source;
            set -gx fish_trace 0;
            /nix/store/XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX-coreutils-9.10/bin/rm /path/to/rc/file;
        "#]].assert_eq(&output);
    }

    #[test]
    fn generate_fish_profile_deactivate() {
        let output = render_deactivate(0);
        expect![[r#"
            set -e ADDED_VAR;
            set -e FLOX_ACTIVATE_START_SERVICES;
            set -e FLOX_ENV;
            set -e FLOX_ENV_CACHE;
            set -e FLOX_ENV_DESCRIPTION;
            set -e FLOX_ENV_DIRS;
            set -e FLOX_ENV_PROJECT;
            set -e FLOX_PROMPT_COLOR_1;
            set -e FLOX_PROMPT_COLOR_2;
            set -e FLOX_PROMPT_ENVIRONMENTS;
            set -e MANPATH;
            set -e PATH;
            set -e QUOTED_VAR;
            set -e _FLOX_ACTIVE_ENVIRONMENTS;
            set -e _FLOX_HOOK_DIFF;
            set -e _FLOX_INVOCATION_TYPE;
            set -e _flox_activations;
            set -gx MODIFIED_VAR MODIFIED_ORIGINAL;
            set -gx DELETED_VAR DELETED_ORIGINAL;
            if isatty 1; source '/interpreter/activate.d/set-prompt.fish'; end;
            /flox_activations profile-scripts-deactivate --shell fish --env '/flox_env' --already-sourced-env-dirs (if set -q _FLOX_SOURCED_PROFILE_SCRIPTS; echo "$_FLOX_SOURCED_PROFILE_SCRIPTS"; else; echo ""; end) | source;
            set -e _activate_d _flox_activate_tracer;
        "#]]
        .assert_eq(&output);
    }

    #[test]
    fn generate_fish_profile_deactivate_traced() {
        // The traced variant is the untraced body wrapped in
        // `set -gx fish_trace 1;` / `set -gx fish_trace 0;`. The body
        // itself is snapshotted by `generate_fish_profile_deactivate`.
        let traced = render_deactivate(2);
        let untraced = render_deactivate(0);
        assert_eq!(
            traced,
            format!("set -gx fish_trace 1;\n{untraced}set -gx fish_trace 0;\n")
        );
    }
}
