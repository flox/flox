_flox_activations="@flox_activations@"

"$_flox_activate_tracer" "$_activate_d/zdotdir/.zprofile" START

# Source /etc/zprofile and "${FLOX_ORIG_ZDOTDIR:-$HOME}/.zprofile" if they exist.
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
_save_FLOX_ACTIVATION_PROFILE_ONLY="$_FLOX_ACTIVATION_PROFILE_ONLY"

restore_saved_vars() {
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
    export _FLOX_ACTIVATION_PROFILE_ONLY="$_save_FLOX_ACTIVATION_PROFILE_ONLY"
    # shellcheck disable=SC1090
    source <("$_flox_activations" set-env-dirs --shell zsh --flox-env "$FLOX_ENV" --env-dirs "${FLOX_ENV_DIRS:-}")
    # shellcheck disable=SC1090
    source <("$_flox_activations" fix-paths --shell zsh --env-dirs "$FLOX_ENV_DIRS" --path "$PATH" --manpath "${MANPATH:-}")
}

if [ -f /etc/zprofile ]
then
    if [ -n "${FLOX_ORIG_ZDOTDIR:-}" ]
    then
        ZDOTDIR="$FLOX_ORIG_ZDOTDIR" FLOX_ORIG_ZDOTDIR= source /etc/zprofile
    else
        ZDOTDIR= source /etc/zprofile
    fi
    restore_saved_vars
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
    restore_saved_vars
fi

# Do not bring in the Nix and Flox environment customizations from this file
# because one of the neighbouring .zshrc or .zlogin files will always be
# sourced after this one.

"$_flox_activate_tracer" "$_activate_d/zdotdir/.zprofile" END
