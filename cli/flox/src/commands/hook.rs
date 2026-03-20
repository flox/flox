use anyhow::{Result, bail};
use bpaf::Bpaf;

#[derive(Bpaf, Clone, Debug)]
pub struct Hook {
    /// Shell to emit hook code for (bash, zsh, fish, tcsh)
    #[bpaf(positional("SHELL"))]
    shell: String,
}

impl Hook {
    pub fn handle(self) -> Result<()> {
        let flox_bin = std::env::current_exe()
            .ok()
            .and_then(|p| p.to_str().map(String::from))
            .unwrap_or_else(|| "flox".to_string());

        let output = match self.shell.as_str() {
            "bash" => bash_hook(&flox_bin),
            "zsh" => zsh_hook(&flox_bin),
            "fish" => fish_hook(&flox_bin),
            "tcsh" => tcsh_hook(&flox_bin),
            other => bail!("unsupported shell: {other}. Supported shells: bash, zsh, fish, tcsh"),
        };

        print!("{output}");
        Ok(())
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
