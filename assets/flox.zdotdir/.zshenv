if [ -f /etc/zshenv ]
then
    if [ -n "${FLOX_ORIG_ZDOTDIR:-}" ]
    then
        ZDOTDIR="$FLOX_ORIG_ZDOTDIR" FLOX_ORIG_ZDOTDIR= source /etc/zshenv
    else
        ZDOTDIR= source /etc/zshenv
    fi
fi

# Explicitly use "export" and don't rely on GLOBAL_EXPORT being set.
# Tell zsh where to store history files.
export HISTFILE=${HISTFILE:-${FLOX_ORIG_ZDOTDIR:-$HOME}/.zsh_history}
# On MacOS Apple have reinvented the wheel, so similary give them a hint.
export SHELL_SESSION_DIR=${SHELL_SESSION_DIR:-${FLOX_ORIG_ZDOTDIR:-$HOME}/.zsh_sessions}

zshenv="${FLOX_ORIG_ZDOTDIR:-$HOME}/.zshenv"
if [ -f "$zshenv" ]
then
    if [ -n "${FLOX_ORIG_ZDOTDIR:-}" ]
    then
        ZDOTDIR="$FLOX_ORIG_ZDOTDIR" FLOX_ORIG_ZDOTDIR= source "$zshenv"
    else
        ZDOTDIR= source "$zshenv"
    fi
fi
