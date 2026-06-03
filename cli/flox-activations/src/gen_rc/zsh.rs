use std::borrow::Cow;
use std::io::Write;
use std::path::PathBuf;

use anyhow::Result;
use flox_core::activate::context::InvocationType;
use flox_core::activate::vars::FLOX_ACTIVATIONS_BIN;
use indoc::{formatdoc, indoc};
use shell_gen::{GenerateShell, Shell, set_unexported_unexpanded, source_file};

use crate::attach_diff::todo_drop_unset;
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
            // (1) drop the `_flox_rehash` precmd hook [outermost only],
            // (2) restore hashing setopts [outermost only],
            // (3) restore FPATH + re-run compinit [every deactivation].
            //
            // Hashing is a global property — it must only be touched on
            // the outermost deactivation.  FPATH/compinit must be
            // recomputed at every deactivation level so that completions
            // from the inner env are immediately removed.
            //
            // Key invariant: `generate_deactivation_statements()` has
            // already restored `FLOX_ENV_DIRS` to its pre-inner-activation
            // value by the time this block runs, so `fix-fpath` sees the
            // remaining active envs and can recompute FPATH correctly.
            if ctx.restore_diff.is_outermost_deactivate() {
                // Block A — outermost only: undo the `_flox_rehash` precmd
                // hook installed by activate.d/zsh.  Guard `unfunction` on
                // the function existing so we don't emit "no such hash table
                // element" if the user removed it themselves mid-session;
                // `add-zsh-hook -d` is already silent for an unregistered
                // hook.
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
            }

            // Block B — every deactivation: restore FPATH and re-run
            // compinit so completions from this env are removed.
            //
            // Outermost case: restore the user's original FPATH directly
            // from the saved value, then re-run their compinit (if they
            // had one before flox).  We honor the pre-flox compinit state
            // rather than running a bare `compinit`, so users with
            // `compinit -u` / `-d <custom>` / no compinit at all don't
            // see surprising behavior on the final deactivation.
            //
            // Inner case: call `fix-fpath` to recompute FPATH from the
            // user's saved base plus the remaining active envs (FLOX_ENV_DIRS
            // has already been restored by the env diff above).  Re-run
            // compinit only when FPATH actually changed.  Dumpfile: prefer
            // `$FLOX_ENV_CACHE/.zcompdump` (outer env's cache, likely a hit
            // from the outer activation); fall back to bare `compinit`.
            // `_FLOX_HOOK_SAVE_COMPINIT_DUMPFILE` is intentionally NOT used
            // here — writing intermediate FPATH state to the user's personal
            // dumpfile would corrupt it.  The save vars are kept (not unset)
            // because the outer activation still needs them for its own
            // deactivation.
            //
            // NOTE on cost: `compinit` rebuilds the completion dump, which
            // can be tens of ms on large `fpath` setups.  For the inner
            // case we skip the rebuild when FPATH is unchanged, and the
            // FLOX_ENV_CACHE dumpfile path makes a cache hit likely.
            if ctx.restore_diff.is_outermost_deactivate() {
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
            } else {
                stmts.push(
                    formatdoc! {r#"
                        if [[ -n "${{_FLOX_HOOK_SAVE_FPATH+set}}" ]]; then
                            _flox_deactivate_old_fpath="$FPATH";
                            source <("{flox_activations}" fix-fpath \
                                --colon-separated-fpath "$_FLOX_HOOK_SAVE_FPATH" \
                                --env-dirs "${{FLOX_ENV_DIRS:-}}");
                            if [[ "$FPATH" != "$_flox_deactivate_old_fpath" ]]; then
                                autoload -U compinit;
                                if [[ -n "${{FLOX_ENV_CACHE:-}}" && -d "${{FLOX_ENV_CACHE}}" ]]; then
                                    compinit -d "${{FLOX_ENV_CACHE}}/.zcompdump";
                                else
                                    compinit;
                                fi;
                            fi;
                            unset _flox_deactivate_old_fpath;
                        fi;"#,
                        flox_activations = ctx.flox_activations.display(),
                    }
                    .to_stmt(),
                );
            }
            // Source the user's profile.deactivate.{common,zsh} scripts
            // for the env being torn down, and remove it from
            // _FLOX_SOURCED_PROFILE_SCRIPTS so stacked activations stay
            // consistent. We bake in the env path at generation time
            // because by now `restore_diff` has either unset `FLOX_ENV`
            // (outermost deactivate) or restored it to the outer env's
            // value (nested), so runtime `$FLOX_ENV` is not a reliable
            // handle on the env we're tearing down. Runs per-env (not
            // gated on outermost) so each env's deactivate hooks fire
            // when that env is torn down.
            stmts.push(
                format!(
                    r#"eval "$('{}' profile-scripts-deactivate --shell {} --env '{}' --already-sourced-env-dirs "${{_FLOX_SOURCED_PROFILE_SCRIPTS:-}}")";"#,
                    FLOX_ACTIVATIONS_BIN.display(),
                    Shell::Zsh,
                    ctx.flox_env.display()
                )
                .to_stmt(),
            );
            // Note that unsetting the prompt depends on `_activate_d` being set.
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
            stmts.push(todo_drop_unset("_FLOX_INVOCATION_TYPE"));
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
        test_deactivate_ctx_inner,
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

    fn render_deactivate_inner() -> String {
        let shell = ShellWithPath::Zsh(PathBuf::from("/bin/zsh"));
        let action = Action::<ZshStartupArgs>::Deactivate(test_deactivate_ctx_inner(shell));
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
            typeset -g _FLOX_INVOCATION_TYPE=interactive;
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
            source /interpreter/activate.d/zsh;
            typeset -g _FLOX_INVOCATION_TYPE=inplace;
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
            unset _flox_activations;
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
            eval "$('/flox_activations' profile-scripts-deactivate --shell zsh --env '/flox_env' --already-sourced-env-dirs "${_FLOX_SOURCED_PROFILE_SCRIPTS:-}")";
            unset _FLOX_INVOCATION_TYPE;
            if [[ -o interactive ]]; then source '/interpreter/activate.d/set-prompt.zsh'; fi;
        "#]]
        .assert_eq(&output);
    }

    /// Verify the inner-deactivation path emits `fix-fpath` (not a direct
    /// FPATH restore), does not emit the outermost-only hashing teardown,
    /// and keeps the save vars for the remaining outer activation.
    #[test]
    fn generate_zsh_profile_commands_deactivate_inner() {
        let output = render_deactivate_inner();
        expect![[r#"
            unset ADDED_VAR;
            export MODIFIED_VAR=MODIFIED_ORIGINAL;
            export _FLOX_ACTIVE_ENVIRONMENTS=/outer/env;
            export DELETED_VAR=DELETED_ORIGINAL;
            unset _FLOX_HOOK_DIFF;
            if [[ -n "${_FLOX_HOOK_SAVE_FPATH+set}" ]]; then
                _flox_deactivate_old_fpath="$FPATH";
                source <("/flox-activations" fix-fpath \
                    --colon-separated-fpath "$_FLOX_HOOK_SAVE_FPATH" \
                    --env-dirs "${FLOX_ENV_DIRS:-}");
                if [[ "$FPATH" != "$_flox_deactivate_old_fpath" ]]; then
                    autoload -U compinit;
                    if [[ -n "${FLOX_ENV_CACHE:-}" && -d "${FLOX_ENV_CACHE}" ]]; then
                        compinit -d "${FLOX_ENV_CACHE}/.zcompdump";
                    else
                        compinit;
                    fi;
                fi;
                unset _flox_deactivate_old_fpath;
            fi;
            eval "$('/flox_activations' profile-scripts-deactivate --shell zsh --env '/flox_env' --already-sourced-env-dirs "${_FLOX_SOURCED_PROFILE_SCRIPTS:-}")";
            unset _FLOX_INVOCATION_TYPE;
            if [[ -o interactive ]]; then source '/interpreter/activate.d/set-prompt.zsh'; fi;
        "#]]
        .assert_eq(&output);
    }
}
