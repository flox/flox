use shell_gen::ShellWithPath;

/// Generate shell-specific hook registration code for auto-activation.
///
/// The generated code registers a prompt hook that calls `flox hook-env`
/// on every prompt, matching the behavior of direnv/mise. The hook only
/// fires in interactive shells (via PROMPT_COMMAND, precmd, fish_prompt),
/// so it naturally does not trigger in non-interactive `bash -c` contexts.
pub fn hook_code_for_shell(shell: &ShellWithPath, flox_bin: &str) -> String {
    match shell {
        ShellWithPath::Bash(_) => bash_hook(flox_bin),
        ShellWithPath::Zsh(_) => zsh_hook(flox_bin),
        ShellWithPath::Fish(_) => fish_hook(flox_bin),
        ShellWithPath::Tcsh(_) => tcsh_hook(flox_bin),
    }
}

fn bash_hook(flox_bin: &str) -> String {
    format!(
        r#"_flox_hook() {{
  local _prev_exit=$?;
  trap '' INT;
  eval "$("{flox_bin}" hook-env --shell bash)";
  trap - INT;
  return $_prev_exit;
}};
if [[ -z "${{PROMPT_COMMAND[*]}}" ]] || [[ ! " ${{PROMPT_COMMAND[*]}} " =~ " _flox_hook " ]]; then
  PROMPT_COMMAND=(_flox_hook "${{PROMPT_COMMAND[@]}}");
fi;
"#
    )
}

fn zsh_hook(flox_bin: &str) -> String {
    format!(
        r#"_flox_hook() {{
  local _prev_exit=$?;
  trap '' INT;
  eval "$("{flox_bin}" hook-env --shell zsh)";
  trap - INT;
  return $_prev_exit;
}};
if (( ! ${{+functions[_flox_hook]}} )) || [[ ! "${{precmd_functions[(r)_flox_hook]}}" == "_flox_hook" ]]; then
  precmd_functions=(_flox_hook $precmd_functions);
fi;
if [[ ! "${{chpwd_functions[(r)_flox_hook]}}" == "_flox_hook" ]]; then
  chpwd_functions=(_flox_hook $chpwd_functions);
fi;
"#
    )
}

fn fish_hook(flox_bin: &str) -> String {
    // Fish's command substitution (flox activate) collapses newlines to spaces,
    // so function definitions must use semicolons as delimiters to survive.
    format!(
        r#"function _flox_hook --on-event fish_prompt; eval ("{flox_bin}" hook-env --shell fish); end;
function _flox_hook_pwd --on-variable PWD; eval ("{flox_bin}" hook-env --shell fish); end;
"#
    )
}

fn tcsh_hook(flox_bin: &str) -> String {
    format!(
        r#"alias precmd 'eval `{flox_bin} hook-env --shell tcsh`';
alias cwdcmd 'eval `{flox_bin} hook-env --shell tcsh`';
"#
    )
}
