# Source /etc/zprofile and "${FLOX_ORIG_ZDOTDIR:-$HOME}/.zprofile" if they exist.
#
# See README.md for more information on the initialization process.

# Save and restore the current tracelevel in the event that sourcing
# bashrc launches an inner nested activation which unsets it.
_save_flox_activate_tracelevel="$_flox_activate_tracelevel"

echo "In zprofile" >&2

if [ -f /etc/zprofile ]
then
    echo "/etc/zprofile existed" >&2
    if [ -n "${FLOX_ORIG_ZDOTDIR:-}" ]
    then
        echo "sourcing /etc/zprofile" >&2
        ZDOTDIR="$FLOX_ORIG_ZDOTDIR" FLOX_ORIG_ZDOTDIR= source /etc/zprofile
    else
        echo "sourcing /etc/zprofile with ZDOTDIR unset" >&2
        ZDOTDIR= source /etc/zprofile
    fi
    export _flox_activate_tracelevel="$_save_flox_activate_tracelevel"
fi

zprofile="${FLOX_ORIG_ZDOTDIR:-$HOME}/.zprofile"
if [ -f "$zprofile" ]
then
    echo "local zprofile existed" >&2
    if [ -n "${FLOX_ORIG_ZDOTDIR:-}" ]
    then
        echo "sourcing local zprofile" >&2
        ZDOTDIR="$FLOX_ORIG_ZDOTDIR" FLOX_ORIG_ZDOTDIR= source "$zprofile"
    else
        echo "sourcing local zprofile with ZDOTDIR unset" >&2
        ZDOTDIR= source "$zprofile"
    fi
    export _flox_activate_tracelevel="$_save_flox_activate_tracelevel"
fi

# Do not bring in the Nix and Flox environment customizations from this file
# because one of the neighbouring .zshrc or .zlogin files will always be
# sourced after this one.
