# shellcheck shell=bash
# shellcheck disable=SC2154
"$_flox_activate_tracer" "$_activate_d/set-prompt.bash" START

# Tweak the (already customized) prompt: add a flox indicator.
_flox_set_prompt() {
  if [ -n "${PS1:-}" ] && [ "${FLOX_PROMPT_ENVIRONMENTS:-}" != "" ]; then
    local _esc="\x1b["
    local colorReset="\[${_esc}0m\]"
    local colorBold="\[${_esc}1m\]"
    local colorPrompt1="\[${_esc}38;5;${FLOX_PROMPT_COLOR_1}m\]"
    local colorPrompt2="\[${_esc}38;5;${FLOX_PROMPT_COLOR_2}m\]"
    local _floxPrompt1="${colorPrompt1}flox"
    local _floxPrompt2="${colorPrompt2}[$FLOX_PROMPT_ENVIRONMENTS]"
    # nixpkgs#bash doesn't have readline, so the prompt gets garbled if we use escapes.
    # Detect if we have readline by checking for progcomp; support for progcomp is
    # disabled when readline is not present, see:
    # https://git.savannah.gnu.org/cgit/bash.git/tree/bashline.c#n23
    # Note that we set colors even if support for progcomp is compiled in, but it is
    # turned off.
    local _flox
    if [[ $(shopt) =~ progcomp ]] && [[ "${NO_COLOR:-0}" == "0" ]]; then
      _flox=$(echo -e -n "${colorBold}${FLOX_PROMPT-$_floxPrompt1} ${_floxPrompt2}${colorReset} ")
    else
      _flox=$(echo -e -n "${FLOX_PROMPT-flox} [$FLOX_PROMPT_ENVIRONMENTS] ")
    fi

    # Start by saving the original value of PS1.
    if [ -z "${FLOX_SAVE_BASH_PS1:=}" ]; then
      export FLOX_SAVE_BASH_PS1="$PS1"
    fi
    case "$FLOX_SAVE_BASH_PS1" in
      # If the prompt contains an embedded newline,
      # then insert the flox indicator immediately after
      # the (first) newline.
      *\\n*) PS1="${FLOX_SAVE_BASH_PS1/\\n/\\n$_flox}" ;;
      *\\012*) PS1="${FLOX_SAVE_BASH_PS1/\\012/\\012$_flox}" ;;

      # Otherwise, prepend the flox indicator.
      *) PS1="$_flox$FLOX_SAVE_BASH_PS1" ;;
    esac
  fi
}

_flox_set_prompt
unset -f _flox_set_prompt

"$_flox_activate_tracer" "$_activate_d/set-prompt.bash" END
