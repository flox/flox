# Source /etc/zshrc and "${FLOX_ORIG_ZDOTDIR:-$HOME}/.zshrc" if they exist
# prior to performing Flox-specific initialization.
#
# See README.md for more information on the initialization process.

# Save and restore the current tracelevel in the event that sourcing
# bashrc launches an inner nested activation which unsets it.
_save_flox_activate_tracelevel="$_flox_activate_tracelevel"

if [ -f /etc/zshrc ]
then
    if [ -n "${FLOX_ORIG_ZDOTDIR:-}" ]
    then
        ZDOTDIR="$FLOX_ORIG_ZDOTDIR" FLOX_ORIG_ZDOTDIR= source /etc/zshrc
    else
        ZDOTDIR= source /etc/zshrc
    fi
    export _flox_activate_tracelevel="$_save_flox_activate_tracelevel"
fi

zshrc="${FLOX_ORIG_ZDOTDIR:-$HOME}/.zshrc"
if [ -f "$zshrc" ]
then
    if [ -n "${FLOX_ORIG_ZDOTDIR:-}" ]
    then
        ZDOTDIR="$FLOX_ORIG_ZDOTDIR" FLOX_ORIG_ZDOTDIR= source "$zshrc"
    else
        ZDOTDIR= source "$zshrc"
    fi
    export _flox_activate_tracelevel="$_save_flox_activate_tracelevel"
fi

# Bring in the Nix and Flox environment customizations, but _not_ if this is
# a login shell. If this is a login shell then the neighbouring .zlogin file
# will be sourced after this one, and we want to delay processing of the flox
# init script to the last possible moment so that no other "rc" files have an
# opportunity to perturb the environment after we've set it up.
[[ -o login ]] || \
  [ -z "$FLOX_ZSH_INIT_SCRIPT" ] || source "$FLOX_ZSH_INIT_SCRIPT"
