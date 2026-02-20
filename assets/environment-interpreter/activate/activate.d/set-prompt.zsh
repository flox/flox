"$_flox_activate_tracer" "$_activate_d/set-prompt.zsh" START

# Tweak the (already customized) prompt: add a flox indicator.

_floxPrompt1="${FLOX_PROMPT-flox}"
_floxPrompt2="[$FLOX_PROMPT_ENVIRONMENTS]"

if [[ "${NO_COLOR:-0}" == "0" ]]; then
  _floxPrompt1="%B%F{${FLOX_PROMPT_COLOR_1}}${_floxPrompt1}%f%b"
  _floxPrompt2="%F{${FLOX_PROMPT_COLOR_2}}${_floxPrompt2}%f"
fi

_flox="${_floxPrompt1} ${_floxPrompt2} "

# Detect prompt frameworks that manage PS1 via precmd hooks. When active,
# modifying PS1 is futile - the framework overwrites it on every command.
# FLOX_PROMPT_ENVIRONMENTS remains exported for use in custom segments.
_flox_external_prompt=0

# Powerlevel10k
if typeset -f _p9k_precmd > /dev/null 2>&1 \
  || [[ -n "${POWERLEVEL9K_CONFIG_FILE:-}" ]]; then
  _flox_external_prompt=1
fi

# Starship
if [[ -n "${STARSHIP_SHELL:-}" ]]; then
  _flox_external_prompt=1
fi

# Pure
if typeset -f prompt_pure_precmd > /dev/null 2>&1; then
  _flox_external_prompt=1
fi

# Oh My Posh
if [[ -n "${POSH_THEME:-}" ]]; then
  _flox_external_prompt=1
fi

if [ -n "$_flox" -a -n "${PS1:-}" -a "${FLOX_PROMPT_ENVIRONMENTS:-}" != "" -a "${_FLOX_SET_PROMPT:-}" != false -a "$_flox_external_prompt" -eq 0 ]; then
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
fi

unset _flox _floxPrompt1 _floxPrompt2 _flox_external_prompt

"$_flox_activate_tracer" "$_activate_d/set-prompt.zsh" END
