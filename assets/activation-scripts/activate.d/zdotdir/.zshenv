# Source /etc/zshenv and "${FLOX_ORIG_ZDOTDIR:-$HOME}/.zshenv" if they exist
# prior to performing Flox-specific initialization.
#
# See README.md for more information on the initialization process.

# Save and restore the current tracelevel in the event that sourcing
# bashrc launches an inner nested activation which unsets it.
_save_flox_activate_tracelevel="$_flox_activate_tracelevel"

echo "In zshenv" >&2

if [ -f /etc/zshenv ]
then
    echo "/etc/zshenv existed" >&2
    if [ -n "${FLOX_ORIG_ZDOTDIR:-}" ]
    then
        echo "sourcing /etc/zshenv" >&2
        ZDOTDIR="$FLOX_ORIG_ZDOTDIR" FLOX_ORIG_ZDOTDIR= source /etc/zshenv
    else
        echo "sourcing /etc/zshenv with ZDOTDIR unset" >&2
        ZDOTDIR= source /etc/zshenv
    fi
    export _flox_activate_tracelevel="$_save_flox_activate_tracelevel"
fi

zshenv="${FLOX_ORIG_ZDOTDIR:-$HOME}/.zshenv"
if [ -f "$zshenv" ]
then
    echo "local zshenv existed" >&2
    if [ -n "${FLOX_ORIG_ZDOTDIR:-}" ]
    then
        echo "sourcing local zshenv" >&2
        ZDOTDIR="$FLOX_ORIG_ZDOTDIR" FLOX_ORIG_ZDOTDIR= source "$zshenv"
    else
        echo "sourcing local zshenv with ZDOTDIR unset" >&2
        ZDOTDIR= source "$zshenv"
    fi
    export _flox_activate_tracelevel="$_save_flox_activate_tracelevel"
fi

# Bring in the Nix and Flox environment customizations, but _not_ if this is
# an interactive or login shell. If the shell is either of these then the
# neighbouring .zshrc or .zlogin files will be sourced after this one, and we
# want to delay processing of the flox init script to the last possible moment
# so that no other "rc" files have an opportunity to perturb the environment
# after we've set it up.
[[ -o interactive ]] || [[ -o login ]] || \
  [ -z "$FLOX_ZSH_INIT_SCRIPT" ] || source "$FLOX_ZSH_INIT_SCRIPT"
