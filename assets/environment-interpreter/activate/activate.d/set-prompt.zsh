"$_flox_activate_tracer" "$_activate_d/set-prompt.zsh" START

# Tweak the (already customized) prompt: add a flox indicator.
_flox_set_prompt() {
  local _floxPrompt1="${FLOX_PROMPT-flox}"
  local _floxPrompt2="[$FLOX_PROMPT_ENVIRONMENTS]"
  if [[ "${NO_COLOR:-0}" == "0" ]]; then
    _floxPrompt1="%B%F{${FLOX_PROMPT_COLOR_1}}${_floxPrompt1}%f%b"
    _floxPrompt2="%F{${FLOX_PROMPT_COLOR_2}}${_floxPrompt2}%f"
  fi
  local _flox="${_floxPrompt1} ${_floxPrompt2} "

  # Start by saving the original value of PS1.
  if [ -z "${FLOX_SAVE_ZSH_PS1:=}" ]; then
    export FLOX_SAVE_ZSH_PS1="$PS1"
  fi
  case "$FLOX_SAVE_ZSH_PS1" in
    # If the prompt contains an embedded newline,
    # then insert the flox indicator immediately after
    # the (first) newline.
    *\\n*) PS1="${FLOX_SAVE_ZSH_PS1/\\n/\\n$_flox}" ;;
    *\\012*) PS1="${FLOX_SAVE_ZSH_PS1/\\012/\\012$_flox}" ;;

    # Otherwise, prepend the flox indicator.
    *) PS1="$_flox$FLOX_SAVE_ZSH_PS1" ;;
  esac

  # TODO: figure out zsh way of setting window and icon title.
}

if [ -n "${PS1:-}" -a "${FLOX_PROMPT_ENVIRONMENTS:-}" != "" ]; then
  _flox_set_prompt
elif [ -n "${FLOX_SAVE_ZSH_PS1:-}" ]; then
  # Restore the prompt when no environments should be in the prompt
  PS1="$FLOX_SAVE_ZSH_PS1"
  unset FLOX_SAVE_ZSH_PS1
fi
unset -f _flox_set_prompt

"$_flox_activate_tracer" "$_activate_d/set-prompt.zsh" END
