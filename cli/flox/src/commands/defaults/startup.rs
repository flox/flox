use std::env;

use flox_core::activate::vars::FLOX_ACTIVE_ENVIRONMENTS_VAR;
use shell_gen::Shell;

use super::DefaultsError;
use crate::utils::active_environments::ActiveEnvironments;
use crate::utils::message;

pub fn preserve_inherited() -> bool {
    match env::var(FLOX_ACTIVE_ENVIRONMENTS_VAR) {
        Err(env::VarError::NotPresent) => false,
        Ok(value) => match value.parse::<ActiveEnvironments>() {
            Ok(active) => active.last_active().is_some(),
            Err(_) => {
                message::warning(
                    "Cannot parse inherited Flox active state; skipping startup selection.",
                );
                true
            },
        },
        Err(env::VarError::NotUnicode(_)) => {
            message::warning(
                "Inherited Flox active state is not UTF-8; skipping startup selection.",
            );
            true
        },
    }
}

pub fn print_setup(name: &str) -> Result<(), DefaultsError> {
    let shell = name
        .parse::<Shell>()
        .map_err(|_| DefaultsError::UnsupportedShell(name.into()))?;
    let (instructions, script) = match shell {
        Shell::Bash => (
            "# Add this block to ~/.bashrc. Your existing login profile should source ~/.bashrc.",
            BASH,
        ),
        Shell::Zsh => (
            "# Add this block to ${ZDOTDIR:-$HOME}/.zshrc, using ZDOTDIR set by your .zshenv.",
            ZSH,
        ),
        Shell::Fish => (
            "# Add this block to $XDG_CONFIG_HOME/fish/config.fish, or ~/.config/fish/config.fish.\n# Use your customized fish configuration location if different.",
            FISH,
        ),
        Shell::Tcsh => (
            "# Add this block to ~/.tcshrc (or ~/.cshrc if no .tcshrc exists). Install it only once.",
            TCSH,
        ),
    };
    println!(
        "{instructions}\n# Ensure flox is on PATH before this block. No files have been changed.\n# Reads the saved selection at each eligible startup; inherited active environments are preserved.\n# Existing activation lines still run. Inner activation failures may be masked by ordinary shell scripts.\n# >>> flox defaults manual startup v1 >>>\n{script}# <<< flox defaults manual startup v1 <<<"
    );
    Ok(())
}

// The selected environment is never interpolated into shell code. FLOX_SHELL
// applies only to the producer, allowing cross-shell children to detect anew.
// Check generation separately from evaluation. Ordinary generated scripts can
// still mask inner failures: shared failure hardening belongs to CLI-250.
const BASH: &str = r#"case $- in
  *i*)
    if _flox_defaults_script="$(command env FLOX_SHELL=bash flox defaults _activate-if-saved)"; then
      if eval "$_flox_defaults_script"; then
        :
      else
        printf '%s\n' 'flox: startup evaluation failed; this shell may have partial activation effects.' >&2
      fi
    else
      printf '%s\n' 'flox: startup activation failed; generated output was not evaluated.' >&2
    fi
    unset _flox_defaults_script
    ;;
esac
"#;

const ZSH: &str = r#"if [[ -o interactive ]]; then
  if _flox_defaults_script="$(command env FLOX_SHELL=zsh flox defaults _activate-if-saved)"; then
    if eval "$_flox_defaults_script"; then
      :
    else
      printf '%s\n' 'flox: startup evaluation failed; this shell may have partial activation effects.' >&2
    fi
  else
    printf '%s\n' 'flox: startup activation failed; generated output was not evaluated.' >&2
  fi
  unset _flox_defaults_script
fi
"#;

// Source a complete private file in fish/tcsh so line splitting and command
// substitution status cannot hide a producer failure or alter generated code.
// mktemp's -t form works with both GNU and BSD implementations.
const FISH: &str = r#"if status is-interactive
  set -l _flox_defaults_file (command mktemp -t flox-defaults.XXXXXXXXXX)
  if test -n "$_flox_defaults_file"; and test -f "$_flox_defaults_file"
    if command env FLOX_SHELL=fish flox defaults _activate-if-saved > "$_flox_defaults_file"
      if source "$_flox_defaults_file"
      else
        printf '%s\n' 'flox: startup evaluation failed; this shell may have partial activation effects.' >&2
      end
    else
      printf '%s\n' 'flox: startup activation failed; generated output was not evaluated.' >&2
    end
    command rm -f -- "$_flox_defaults_file"
  else
    printf '%s\n' 'flox: could not create a temporary startup script; activation skipped.' >&2
  end
  set -e _flox_defaults_file
end
"#;

const TCSH: &str = r#"if ($?prompt) then
  set _flox_defaults_file = "`\mktemp -t flox-defaults.XXXXXXXXXX`"
  if ("$_flox_defaults_file" != "" && -f "$_flox_defaults_file") then
    \env FLOX_SHELL=tcsh flox defaults _activate-if-saved >! "$_flox_defaults_file"
    if ($status == 0) then
      source "$_flox_defaults_file"
      if ($status != 0) then
        echo 'flox: startup evaluation failed; this shell may have partial activation effects.' >& /dev/stderr
      endif
    else
      echo 'flox: startup activation failed; generated output was not evaluated.' >& /dev/stderr
    endif
    \rm -f -- "$_flox_defaults_file"
  else
    echo 'flox: could not create a temporary startup script; activation skipped.' >& /dev/stderr
  endif
  unset _flox_defaults_file
endif
"#;
