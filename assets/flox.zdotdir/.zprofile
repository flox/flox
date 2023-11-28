if [ -f /etc/zprofile ]
then
    if [ -n "${FLOX_ORIG_ZDOTDIR:-}" ]
    then
        ZDOTDIR="$FLOX_ORIG_ZDOTDIR" FLOX_ORIG_ZDOTDIR= source /etc/zprofile
    else
        ZDOTDIR= source /etc/zprofile
    fi
fi

zprofile="${FLOX_ORIG_ZDOTDIR:-$HOME}/.zprofile"
if [ -f "$zprofile" ]
then
    if [ -n "${FLOX_ORIG_ZDOTDIR:-}" ]
    then
        ZDOTDIR="$FLOX_ORIG_ZDOTDIR" FLOX_ORIG_ZDOTDIR= source "$zprofile"
    else
        ZDOTDIR= source "$zprofile"
    fi
fi
