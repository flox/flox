//! Shell-specific hook registration code for auto-activation.
//!
//! The generated code registers a prompt hook that calls `flox hook-env`
//! on every prompt, matching the behavior of direnv. The hook only
//! fires in interactive shells (via PROMPT_COMMAND, precmd, fish_prompt),
//! so it naturally does not trigger in non-interactive (e.g. `bash -c`) contexts.
//!
//! Each hook passes the interactive shell's PID (`$$` / `$fish_pid`) so
//! `hook-env` can find this shell's prompt-hook action file, and the invocation
//! type so `hook-env` can emit the right deactivation script.
//! `_FLOX_INVOCATION_TYPE` is a shell-local set during activation and unset on
//! deactivation, so the hook — which keeps firing afterwards — defaults it to
//! `inplace` when it is unset. bash and zsh express that default inline; fish
//! factors it into the `_flox_invocation_type` helper function; tcsh can't
//! reference a possibly-unset variable without erroring, so it guards with `$?`
//! and a throwaway variable (see `tcsh_hook`). `--invocation-type` is still
//! optional on `hook-env` as a defensive measure.
//!
//! Each hook also exports [`PROMPT_HOOK_VERSION_ENV`] =
//! [`PROMPT_HOOK_VERSION`] at registration time (top level, so it is set
//! before the first prompt). It is exported, unlike `_FLOX_INVOCATION_TYPE`, so
//! a subprocess such as `flox deactivate` can confirm a compatible hook is set
//! up before writing an action file the hook would otherwise never consume.

use flox_core::hook_actions::{PROMPT_HOOK_VERSION, PROMPT_HOOK_VERSION_ENV};
use indoc::formatdoc;

pub fn bash_hook(flox_bin: &str) -> String {
    formatdoc!(
        r#"
        export {PROMPT_HOOK_VERSION_ENV}={PROMPT_HOOK_VERSION};
        _flox_hook() {{
          local _prev_exit=$?;
          local _flox_vars;
          _flox_vars="$("{flox_bin}" hook-env --shell bash --shell-pid $$ --invocation-type "${{_FLOX_INVOCATION_TYPE:-inplace}}")";
          trap -- '' SIGINT;
          eval "$_flox_vars";
          trap - SIGINT;
          return $_prev_exit;
        }};
        if [[ ";${{PROMPT_COMMAND[*]:-}};" != *";_flox_hook;"* ]]; then
          if [[ "$(declare -p PROMPT_COMMAND 2>&1)" == "declare -a"* ]]; then
            PROMPT_COMMAND=(_flox_hook "${{PROMPT_COMMAND[@]}}");
          else
            PROMPT_COMMAND="_flox_hook${{PROMPT_COMMAND:+;$PROMPT_COMMAND}}";
          fi;
        fi;
        "#
    )
}

// Unlike bash, zsh restores $? before calling each precmd function
// independently, so we don't need to save/restore it ourselves.
pub fn zsh_hook(flox_bin: &str) -> String {
    formatdoc!(
        r#"
        export {PROMPT_HOOK_VERSION_ENV}={PROMPT_HOOK_VERSION};
        _flox_hook() {{
          local _flox_vars;
          _flox_vars="$("{flox_bin}" hook-env --shell zsh --shell-pid $$ --invocation-type "${{_FLOX_INVOCATION_TYPE:-inplace}}")";
          trap -- '' SIGINT;
          eval "$_flox_vars";
          trap - SIGINT;
        }};
        typeset -ag precmd_functions;
        if (( ! ${{+functions[_flox_hook]}} )) || (( ! ${{precmd_functions[(I)_flox_hook]}} )); then
          precmd_functions=(_flox_hook $precmd_functions);
        fi;
        typeset -ag chpwd_functions;
        if (( ! ${{chpwd_functions[(I)_flox_hook]}} )); then
          chpwd_functions=(_flox_hook $chpwd_functions);
        fi;
        "#
    )
}

pub fn fish_hook(flox_bin: &str) -> String {
    // Fish's command substitution (flox activate) collapses newlines to spaces,
    // so semicolons are required as statement delimiters to survive. The
    // newlines are kept for readability — fish treats them as whitespace.
    //
    // Fish doesn't parse nested `function...end` blocks properly when the
    // code arrives via eval with collapsed newlines, so we can't nest the
    // PWD hook inside the prompt handler like direnv does. Instead, all
    // three functions are defined at the top level, and a flag variable
    // (_flox_pwd_hook_active) gates the PWD hook's behavior.
    //
    // The mode is read at runtime from $FLOX_AUTO_ACTIVATE_FISH_MODE,
    // matching direnv's `direnv_fish_mode` pattern. This lets the user
    // change modes without re-activating. Values:
    //   - eval_on_arrow (default when unset): PWD hook fires immediately
    //     during interactive prompt use but not during command execution.
    //   - eval_after_arrow: PWD hook sets a flag; evaluation is deferred
    //     until before the next command executes (fish_preexec).
    //   - disable_arrow: no PWD reaction; only prompt-based evaluation.
    formatdoc!(
        r#"
        set -gx {PROMPT_HOOK_VERSION_ENV} {PROMPT_HOOK_VERSION};
        function _flox_invocation_type;
            test -n "$_FLOX_INVOCATION_TYPE"; and echo $_FLOX_INVOCATION_TYPE; or echo inplace;
        end;
        function _flox_hook --on-event fish_prompt;
            "{flox_bin}" hook-env --shell fish --shell-pid $fish_pid --invocation-type (_flox_invocation_type) | source;
            if test "$FLOX_AUTO_ACTIVATE_FISH_MODE" != "disable_arrow";
                set -g _flox_pwd_hook_active 1;
            end;
        end;
        function _flox_hook_pwd --on-variable PWD;
            if set -q _flox_pwd_hook_active;
                if test "$FLOX_AUTO_ACTIVATE_FISH_MODE" = "eval_after_arrow";
                    set -g _flox_env_again 0;
                else;
                    "{flox_bin}" hook-env --shell fish --shell-pid $fish_pid --invocation-type (_flox_invocation_type) | source;
                end;
            end;
        end;
        function _flox_hook_preexec --on-event fish_preexec;
            if set -q _flox_env_again;
                set -e _flox_env_again;
                "{flox_bin}" hook-env --shell fish --shell-pid $fish_pid --invocation-type (_flox_invocation_type) | source;
            end;
            set -e _flox_pwd_hook_active;
        end;
        "#
    )
}

// Set both precmd and cwdcmd so we get pushd/popd behavior similar to what we have for zsh.
//
// Passing --invocation-type in tcsh is awkward: referencing an unset variable is
// a hard error, so a possibly-unset `_FLOX_INVOCATION_TYPE` can't be expanded
// directly the way bash/zsh/fish do with a default. Guard it with `$?` instead:
// seed a throwaway `_flox_invocation_type` with the `inplace` default, overwrite
// it from `_FLOX_INVOCATION_TYPE` only when that is set, pass it, then unset it
// so it doesn't linger. `inplace` is the right default because the prompt hook
// only ever deactivates in place.
pub fn tcsh_hook(flox_bin: &str) -> String {
    // A tcsh alias body must be a single line, so assemble the statements here
    // and join them with "; " rather than writing one long string literal.
    let hook = [
        "set _flox_invocation_type=inplace".to_string(),
        r#"if ( $?_FLOX_INVOCATION_TYPE ) set _flox_invocation_type="$_FLOX_INVOCATION_TYPE""#.to_string(),
        format!(
            r#"eval "`{flox_bin} hook-env --shell tcsh --shell-pid $$ --invocation-type "$_flox_invocation_type"`""#
        ),
        "unset _flox_invocation_type".to_string(),
    ]
    .join("; ");
    formatdoc!(
        r#"
        setenv {PROMPT_HOOK_VERSION_ENV} {PROMPT_HOOK_VERSION};
        alias precmd '{hook}';
        alias cwdcmd '{hook}';
        "#
    )
}
