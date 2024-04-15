if [ -f /etc/zshenv ]
then
    if [ -n "${FLOX_ORIG_ZDOTDIR:-}" ]
    then
        ZDOTDIR="$FLOX_ORIG_ZDOTDIR" FLOX_ORIG_ZDOTDIR= source /etc/zshenv
    else
        ZDOTDIR= source /etc/zshenv
    fi
fi

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

# Bring in the Nix and Flox environment customizations.
[ -z "$FLOX_ZSH_INIT_SCRIPT" ] || source "$FLOX_ZSH_INIT_SCRIPT"
