# Tweak the (already customized) prompt: add a flox indicator.
_esc="\x1b["
colorReset="\[${_esc}0m\]"
colorBold="\[${_esc}1m\]"
colorPrompt1="\[${_esc}38;5;${FLOX_PROMPT_COLOR_1}m\]"
colorPrompt2="\[${_esc}38;5;${FLOX_PROMPT_COLOR_2}m\]"
_floxPrompt1="${colorPrompt1}flox"
_floxPrompt2="${colorPrompt2}[$FLOX_PROMPT_ENVIRONMENTS]"
_flox=$(echo -e -n "${colorBold}${FLOX_PROMPT-$_floxPrompt1} ${_floxPrompt2}${colorReset} ")
unset _esc colorReset colorBold colorPrompt1 colorPrompt2 _floxPrompt1 _floxPrompt2

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

    # Older versions of bash don't support the "@P" operator
    # so attempt the eval first before proceeding for real.
    if eval ': "${_flox@P}"' 2> /dev/null
    then
        # Remove all color and escape sequences from $_flox
        # before adding to window titles and icon names.
        _flox=$(echo "${_flox@P}" | $_ansifilter)

        # Prepend the flox indicator to window titles and icon names.
        PS1="${PS1//\\e]0;/\\e]0;$_flox}"
        PS1="${PS1//\\e]1;/\\e]1;$_flox}"
        PS1="${PS1//\\e]2;/\\e]2;$_flox}"

        PS1="${PS1//\\033]0;/\\033]0;$_flox}"
        PS1="${PS1//\\033]1;/\\033]1;$_flox}"
        PS1="${PS1//\\033]2;/\\033]2;$_flox}"
    fi
fi

unset _flox
