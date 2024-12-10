# Source /etc/zshenv and "${FLOX_ORIG_ZDOTDIR:-$HOME}/.zshenv" if they exist
# prior to performing Flox-specific initialization.
#
# See README.md for more information on the initialization process.

# Save and restore the current tracelevel in the event that sourcing
# bashrc launches an inner nested activation which unsets it.
_save_flox_activate_tracelevel="$_flox_activate_tracelevel"
_save_FLOX_ACTIVATION_STATE_DIR="$_FLOX_ACTIVATION_STATE_DIR"
_save_FLOX_ENV="$FLOX_ENV"
_save_FLOX_ORIG_ZDOTDIR="$FLOX_ORIG_ZDOTDIR"
_save_ZDOTDIR="$ZDOTDIR"
_save_activate_d="$_activate_d"
_save_FLOX_ZSH_INIT_SCRIPT="$FLOX_ZSH_INIT_SCRIPT"
_save_FLOX_RESTORE_PATH="$_FLOX_RESTORE_PATH"
_save_FLOX_RESTORE_MANPATH="$_FLOX_RESTORE_MANPATH"
_save_FLOX_ACTIVATION_PROFILE_ONLY="$_FLOX_ACTIVATION_PROFILE_ONLY"

restore_saved_vars() {
    export _flox_activate_tracelevel="$_save_flox_activate_tracelevel"
    export FLOX_ENV="$_save_FLOX_ENV"
    export FLOX_ORIG_ZDOTDIR="$_save_FLOX_ORIG_ZDOTDIR"
    export ZDOTDIR="$_save_ZDOTDIR"
    export _activate_d="$_save_activate_d"
    export FLOX_ZSH_INIT_SCRIPT="$_save_FLOX_ZSH_INIT_SCRIPT"
    export _FLOX_ACTIVATION_STATE_DIR="$_save_FLOX_ACTIVATION_STATE_DIR"
    export _FLOX_RESTORE_PATH="$_save_FLOX_RESTORE_PATH"
    export _FLOX_RESTORE_MANPATH="$_save_FLOX_RESTORE_MANPATH"
    export _FLOX_ACTIVATION_PROFILE_ONLY="$_save_FLOX_ACTIVATION_PROFILE_ONLY"
}

if [ -f /etc/zshenv ]
then
    if [ -n "${FLOX_ORIG_ZDOTDIR:-}" ]
    then
        ZDOTDIR="$FLOX_ORIG_ZDOTDIR" FLOX_ORIG_ZDOTDIR= source /etc/zshenv
    else
        ZDOTDIR= source /etc/zshenv
    fi
    restore_saved_vars
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
    restore_saved_vars
fi

# Bring in the Nix and Flox environment customizations, but _not_ if this is
# an interactive or login shell. If the shell is either of these then the
# neighbouring .zshrc or .zlogin files will be sourced after this one, and we
# want to delay processing of the flox init script to the last possible moment
# so that no other "rc" files have an opportunity to perturb the environment
# after we've set it up.
[[ -o interactive ]] || [[ -o login ]] || \
  [ -z "$FLOX_ZSH_INIT_SCRIPT" ] || source "$FLOX_ZSH_INIT_SCRIPT"
