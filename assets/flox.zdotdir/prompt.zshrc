
# Tweak the (already customized) prompt: add a flox indicator.
_floxPrompt1="%F{${FLOX_PROMPT_COLOR_1}}flox"
_floxPrompt2="%F{$FLOX_PROMPT_COLOR_2}[$FLOX_PROMPT_ENVIRONMENTS]"
_flox="%B${FLOX_PROMPT-$_floxPrompt1} ${_floxPrompt2}%f%b "


if [ -n "$_flox" -a -n "${PS1:-}" ]
then
    # Start by saving the original value of PS1.
    if [ -z "$FLOX_SAVE_PS1" ]; then
        export FLOX_SAVE_PS1="$PS1"
    fi
    case "$FLOX_SAVE_PS1" in
        # If the prompt contains an embedded newline,
        # then insert the flox indicator immediately after
        # the (first) newline.
        *\\n*)      PS1="${FLOX_SAVE_PS1/\\n/\\n$_flox}";;
        *\\012*)    PS1="${FLOX_SAVE_PS1/\\012/\\012$_flox}";;

        # Otherwise, prepend the flox indicator.
        *)          PS1="$_flox$FLOX_SAVE_PS1";;
    esac

    # TODO: figure out zsh way of setting window and icon title.
fi

unset _flox _floxPrompt1 _floxPrompt2
