# Source /etc/zlogin and "${FLOX_ORIG_ZDOTDIR:-$HOME}/.zlogin" if they exist
# prior to performing Flox-specific initialization.
#
# See README.md for more information on the initialization process.

# Save and restore the current tracelevel in the event that sourcing
# bashrc launches an inner nested activation which unsets it.
_save_flox_activate_tracelevel="$_flox_activate_tracelevel"

echo "in zlogin" >&2

if [ -f /etc/zlogin ]
then
    echo "/etc/zlogin existed" >&2
    if [ -n "${FLOX_ORIG_ZDOTDIR:-}" ]
    then
        echo "sourcing /etc/zlogin" >&2
        ZDOTDIR="$FLOX_ORIG_ZDOTDIR" FLOX_ORIG_ZDOTDIR= source /etc/zlogin
    else
        echo "sourcing /etc/zlogin without ZDOTDIR set" >&2
        ZDOTDIR= source /etc/zlogin
    fi
    export _flox_activate_tracelevel="$_save_flox_activate_tracelevel"
fi

zlogin="${FLOX_ORIG_ZDOTDIR:-$HOME}/.zlogin"
if [ -f "$zlogin" ]
then
    echo "local zlogin existed" >&2
    if [ -n "${FLOX_ORIG_ZDOTDIR:-}" ]
    then
        echo "sourcing local zlogin" >&2
        ZDOTDIR="$FLOX_ORIG_ZDOTDIR" FLOX_ORIG_ZDOTDIR= source "$zlogin"
    else
        echo "sourcing local zlogin without ZDOTDIR set" >&2
        ZDOTDIR= source "$zlogin"
    fi
    export _flox_activate_tracelevel="$_save_flox_activate_tracelevel"
fi

# Bring in the Nix and Flox environment customizations.
[ -z "$FLOX_ZSH_INIT_SCRIPT" ] || source "$FLOX_ZSH_INIT_SCRIPT"
