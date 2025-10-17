_flox_activations="@flox_activations@"

"$_flox_activate_tracer" "$_activate_d/zdotdir/.zshenv" START

# Source /etc/zshenv and "${FLOX_ORIG_ZDOTDIR:-$HOME}/.zshenv" if they exist
# prior to performing Flox-specific initialization.
#
# See README.md for more information on the initialization process.

# Save and restore the current tracelevel in the event that sourcing
# bashrc launches an inner nested activation which unsets it.
_save_flox_activate_tracelevel="$_flox_activate_tracelevel"
_save_FLOX_ACTIVATION_STATE_DIR="$_FLOX_ACTIVATION_STATE_DIR"
_save_FLOX_ENV="$FLOX_ENV"
_save_FLOX_ENV_CACHE="$FLOX_ENV_CACHE"
_save_FLOX_ENV_PROJECT="$FLOX_ENV_PROJECT"
_save_FLOX_ENV_DESCRIPTION="$FLOX_ENV_DESCRIPTION"
_save_FLOX_ORIG_ZDOTDIR="$FLOX_ORIG_ZDOTDIR"
_save_ZDOTDIR="$ZDOTDIR"
_save_activate_d="$_activate_d"
_save_flox_activate_tracer="$_flox_activate_tracer"
_save_FLOX_ZSH_INIT_SCRIPT="$FLOX_ZSH_INIT_SCRIPT"

restore_saved_vars() {
    unset _flox_sourcing_rc
    export _flox_activate_tracelevel="$_save_flox_activate_tracelevel"
    export FLOX_ENV="$_save_FLOX_ENV"
    export FLOX_ENV_CACHE="$_save_FLOX_ENV_CACHE"
    export FLOX_ENV_PROJECT="$_save_FLOX_ENV_PROJECT"
    export FLOX_ENV_DESCRIPTION="$_save_FLOX_ENV_DESCRIPTION"
    export FLOX_ORIG_ZDOTDIR="$_save_FLOX_ORIG_ZDOTDIR"
    export ZDOTDIR="$_save_ZDOTDIR"
    export _activate_d="$_save_activate_d"
    export _flox_activate_tracer="$_save_flox_activate_tracer"
    export FLOX_ZSH_INIT_SCRIPT="$_save_FLOX_ZSH_INIT_SCRIPT"
    export _FLOX_ACTIVATION_STATE_DIR="$_save_FLOX_ACTIVATION_STATE_DIR"
}

if [ -f /etc/zshenv ]
then
    export _flox_sourcing_rc=1
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
    export _flox_sourcing_rc=1
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

"$_flox_activate_tracer" "$_activate_d/zdotdir/.zshenv" END
