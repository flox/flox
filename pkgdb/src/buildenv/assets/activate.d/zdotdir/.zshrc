# Source /etc/zshrc and "${FLOX_ORIG_ZDOTDIR:-$HOME}/.zshrc" if they exist
# prior to performing Flox-specific initialization.
#
# See README.md for more information on the initialization process.

zshrc="${FLOX_ORIG_ZDOTDIR:-$HOME}/.zshrc"
flox_zdotdir="$ZDOTDIR"

# This is the only file in which we need to perform flox actions so take this
# opportunity to restore the user's original $ZDOTDIR if defined, otherwise
# remove it from the environment.
# zlogin hasn't been sourced yet, but it will be sourced as it normally would
# after we reset ZDOTDIR.
if [ -n "${FLOX_ORIG_ZDOTDIR:-}" ]
then
    export ZDOTDIR="$FLOX_ORIG_ZDOTDIR"
    unset FLOX_ORIG_ZDOTDIR
else
    unset ZDOTDIR
fi

if [ -f /etc/zshrc ]
then
    source /etc/zshrc
fi

# Do all of the usual initializations.
if [ -f "$zshrc" ]
then
    source "$zshrc"
fi

# Bring in the Nix and Flox environment customizations.
[ -z "$FLOX_ZSH_INIT_SCRIPT" ] || source "$FLOX_ZSH_INIT_SCRIPT"
