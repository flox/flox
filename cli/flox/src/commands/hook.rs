use anyhow::{Result, bail};
use bpaf::Bpaf;
use shell_gen::ShellWithPath;

use super::activate::Activate;

#[derive(Bpaf, Clone, Debug)]
pub struct Hook {
    /// Shell to emit hook code for (bash, zsh, fish, tcsh).
    /// Auto-detected from the current shell if omitted.
    #[bpaf(positional("SHELL"), optional)]
    shell: Option<String>,
}

impl Hook {
    pub fn handle(self) -> Result<()> {
        let flox_bin = std::env::current_exe()
            .ok()
            .and_then(|p| p.to_str().map(String::from))
            .unwrap_or_else(|| "flox".to_string());

        let output = match self.shell {
            Some(ref shell_name) => match shell_name.as_str() {
                "bash" => bash_hook(&flox_bin),
                "zsh" => zsh_hook(&flox_bin),
                "fish" => fish_hook(&flox_bin),
                "tcsh" => tcsh_hook(&flox_bin),
                other => {
                    bail!("unsupported shell: {other}. Supported shells: bash, zsh, fish, tcsh")
                },
            },
            None => {
                let shell = Activate::detect_shell_for_in_place()?;
                hook_code_for_shell(&shell, &flox_bin)
            },
        };

        print!("{output}");
        Ok(())
    }
}

/// Generate hook code for a given shell.
///
/// This is used by both `flox hook` and `flox activate` (eval mode)
/// to emit auto-activation hook registration code.
pub(crate) fn hook_code_for_shell(shell: &ShellWithPath, flox_bin: &str) -> String {
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
flox() {{
  if [ "$1" = "deactivate" ]; then
    eval "$(command "{flox_bin}" deactivate --shell bash "${{@:2}}")";
  else
    command "{flox_bin}" "$@";
  fi;
}};
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
flox() {{
  if [ "$1" = "deactivate" ]; then
    eval "$(command "{flox_bin}" deactivate --shell zsh "${{@:2}}")";
  else
    command "{flox_bin}" "$@";
  fi;
}};
"#
    )
}

fn fish_hook(flox_bin: &str) -> String {
    format!(
        r#"function _flox_hook --on-event fish_prompt
  eval ("{flox_bin}" hook-env --shell fish)
end;
function _flox_hook_pwd --on-variable PWD
  eval ("{flox_bin}" hook-env --shell fish)
end;
function flox --wraps={flox_bin}
  if test (count $argv) -ge 1; and test "$argv[1]" = "deactivate"
    eval (command "{flox_bin}" deactivate --shell fish $argv[2..])
  else
    command "{flox_bin}" $argv
  end
end;
"#
    )
}

fn tcsh_hook(flox_bin: &str) -> String {
    format!(
        r#"alias precmd 'eval `{flox_bin} hook-env --shell tcsh`';
alias cwdcmd 'eval `{flox_bin} hook-env --shell tcsh`';
alias flox 'if (\!:1 == deactivate) then; eval `{flox_bin} deactivate --shell tcsh \!:2*`; else; command {flox_bin} \!*; endif';
"#
    )
}
