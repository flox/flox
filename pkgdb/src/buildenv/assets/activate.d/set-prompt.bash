# shellcheck shell=bash disable=all
# Tweak the (already customized) prompt: add a flox indicator.

_esc="\x1b["
colorReset="\[${_esc}0m\]"
colorBold="\[${_esc}1m\]"
colorPrompt1="\[${_esc}38;5;${FLOX_PROMPT_COLOR_1}m\]"
colorPrompt2="\[${_esc}38;5;${FLOX_PROMPT_COLOR_2}m\]"
_floxPrompt1="${colorPrompt1}flox"
_floxPrompt2="${colorPrompt2}[$FLOX_PROMPT_ENVIRONMENTS]"
# nixpkgs#bash doesn't have readline, so the prompt gets garbled if we use escapes.
# Detect if we have readline by checking for progcomp; support for progcomp is
# disabled when readline is not present, see:
# https://git.savannah.gnu.org/cgit/bash.git/tree/bashline.c#n23
# Note that we set colors even if support for progcomp is compiled in, but it is
# turned off.
if [[ $(shopt) =~ progcomp ]] && [[ "${NO_COLOR:-0}" == "0" ]]; then
  _flox=$(echo -e -n "${colorBold}${FLOX_PROMPT-$_floxPrompt1} ${_floxPrompt2}${colorReset} ")
else
  _flox=$(echo -e -n "${FLOX_PROMPT-flox} [$FLOX_PROMPT_ENVIRONMENTS] ")
fi

unset _esc colorReset colorBold colorPrompt1 colorPrompt2 _floxPrompt1 _floxPrompt2

if [ -n "$_flox" ] && [ -n "${PS1:-}" ] && [ "${FLOX_PROMPT_ENVIRONMENTS:-}" != "" ]; then
  # Start by saving the original value of PS1.
  if [ -z "$FLOX_SAVE_BASH_PS1" ]; then
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

unset _flox
