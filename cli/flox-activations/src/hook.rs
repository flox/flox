//! Shell-specific hook registration code for auto-activation.
//!
//! The generated code registers a prompt hook that calls `flox hook-env`
//! on every prompt, matching the behavior of direnv. The hook only
//! fires in interactive shells (via PROMPT_COMMAND, precmd, fish_prompt),
//! so it naturally does not trigger in non-interactive (e.g. `bash -c`) contexts.

use indoc::formatdoc;

pub fn bash_hook(flox_bin: &str) -> String {
    formatdoc!(
        r#"
        _flox_hook() {{
          local _prev_exit=$?;
          if [ -z "${{_FLOX_HOOK_PARENT_PS1+x}}" ]; then export _FLOX_HOOK_PARENT_PS1="$PS1"; fi;
          local _flox_vars;
          _flox_vars="$("{flox_bin}" hook-env --shell bash)";
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
        flox() {{
          if [ "$1" = "deactivate" ]; then
            local _flox_eval;
            _flox_eval="$("{flox_bin}" deactivate --shell-eval --shell bash "${{@:2}}")";
            if [ $? -eq 0 ]; then eval "$_flox_eval"; else return $?; fi;
          else
            "{flox_bin}" "$@";
          fi;
        }};
        "#
    )
}

// Unlike bash, zsh restores $? before calling each precmd function
// independently, so we don't need to save/restore it ourselves.
pub fn zsh_hook(flox_bin: &str) -> String {
    formatdoc!(
        r#"
        _flox_hook() {{
          if [ -z "${{_FLOX_HOOK_PARENT_PS1+x}}" ]; then export _FLOX_HOOK_PARENT_PS1="$PS1"; fi;
          local _flox_vars;
          _flox_vars="$("{flox_bin}" hook-env --shell zsh)";
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
        flox() {{
          if [ "$1" = "deactivate" ]; then
            local _flox_eval;
            _flox_eval="$("{flox_bin}" deactivate --shell-eval --shell zsh "${{@:2}}")";
            if [ $? -eq 0 ]; then eval "$_flox_eval"; else return $?; fi;
          else
            "{flox_bin}" "$@";
          fi;
        }};
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
        function _flox_hook --on-event fish_prompt;
            "{flox_bin}" hook-env --shell fish | source;
            if test "$FLOX_AUTO_ACTIVATE_FISH_MODE" != "disable_arrow";
                set -g _flox_pwd_hook_active 1;
            end;
        end;
        function _flox_hook_pwd --on-variable PWD;
            if set -q _flox_pwd_hook_active;
                if test "$FLOX_AUTO_ACTIVATE_FISH_MODE" = "eval_after_arrow";
                    set -g _flox_env_again 0;
                else;
                    "{flox_bin}" hook-env --shell fish | source;
                end;
            end;
        end;
        function _flox_hook_preexec --on-event fish_preexec;
            if set -q _flox_env_again;
                set -e _flox_env_again;
                "{flox_bin}" hook-env --shell fish | source;
            end;
            set -e _flox_pwd_hook_active;
        end;
        "#
    )
}

// Set both precmd and cwdcmd so we get pushd/popd behavior similar to what we have for zsh
pub fn tcsh_hook(flox_bin: &str) -> String {
    formatdoc!(
        r#"
        alias precmd 'eval `{flox_bin} hook-env --shell tcsh`';
        alias cwdcmd 'eval `{flox_bin} hook-env --shell tcsh`';
        "#
    )
}
