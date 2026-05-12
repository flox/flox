//! Shell-specific hook registration code for auto-activation.
//!
//! The generated code registers a prompt hook that calls `flox hook-env`
//! on every prompt, matching the behavior of direnv. The hook only
//! fires in interactive shells (via PROMPT_COMMAND, precmd, fish_prompt),
//! so it naturally does not trigger in non-interactive (e.g. `bash -c`) contexts.

pub fn bash_hook(flox_bin: &str) -> String {
    format!(
        r#"_flox_hook() {{
  local _prev_exit=$?;
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
"#
    )
}

pub fn zsh_hook(flox_bin: &str) -> String {
    format!(
        r#"_flox_hook() {{
  local _prev_exit=$?;
  local _flox_vars;
  _flox_vars="$("{flox_bin}" hook-env --shell zsh)";
  trap -- '' SIGINT;
  eval "$_flox_vars";
  trap - SIGINT;
  return $_prev_exit;
}};
typeset -ag precmd_functions;
if (( ! ${{+functions[_flox_hook]}} )) || [[ ! "${{precmd_functions[(r)_flox_hook]}}" == "_flox_hook" ]]; then
  precmd_functions=(_flox_hook $precmd_functions);
fi;
typeset -ag chpwd_functions;
if [[ ! "${{chpwd_functions[(r)_flox_hook]}}" == "_flox_hook" ]]; then
  chpwd_functions=(_flox_hook $chpwd_functions);
fi;
"#
    )
}

pub fn fish_hook(flox_bin: &str) -> String {
    // Fish's command substitution (flox activate) collapses newlines to spaces,
    // so function definitions must use semicolons as delimiters to survive.
    format!(
        r#"function _flox_hook --on-event fish_prompt; "{flox_bin}" hook-env --shell fish | source; end;
function _flox_hook_pwd --on-variable PWD; "{flox_bin}" hook-env --shell fish | source; end;
"#
    )
}

pub fn tcsh_hook(flox_bin: &str) -> String {
    format!(
        r#"alias precmd 'eval `{flox_bin} hook-env --shell tcsh`';
alias cwdcmd 'eval `{flox_bin} hook-env --shell tcsh`';
"#
    )
}
