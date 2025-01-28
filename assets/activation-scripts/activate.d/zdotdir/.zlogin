_flox_activations="@flox_activations@"

"$_flox_activate_tracer" "$activate_d/zdotdir/.zlogin" START

# Source /etc/zlogin and "${FLOX_ORIG_ZDOTDIR:-$HOME}/.zlogin" if they exist
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
_save_flox_activate_tracer="$_flox_activate_tracer"
_save_FLOX_ZSH_INIT_SCRIPT="$FLOX_ZSH_INIT_SCRIPT"
_save_FLOX_ACTIVATION_PROFILE_ONLY="$_FLOX_ACTIVATION_PROFILE_ONLY"

restore_saved_vars() {
    export _flox_activate_tracelevel="$_save_flox_activate_tracelevel"
    export FLOX_ENV="$_save_FLOX_ENV"
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

if [ -f /etc/zlogin ]
then
    if [ -n "${FLOX_ORIG_ZDOTDIR:-}" ]
    then
        ZDOTDIR="$FLOX_ORIG_ZDOTDIR" FLOX_ORIG_ZDOTDIR= source /etc/zlogin
    else
        ZDOTDIR= source /etc/zlogin
    fi
    restore_saved_vars
fi

zlogin="${FLOX_ORIG_ZDOTDIR:-$HOME}/.zlogin"
if [ -f "$zlogin" ]
then
    if [ -n "${FLOX_ORIG_ZDOTDIR:-}" ]
    then
        ZDOTDIR="$FLOX_ORIG_ZDOTDIR" FLOX_ORIG_ZDOTDIR= source "$zlogin"
    else
        ZDOTDIR= source "$zlogin"
    fi
    restore_saved_vars
fi

# Bring in the Nix and Flox environment customizations.
[ -z "$FLOX_ZSH_INIT_SCRIPT" ] || source "$FLOX_ZSH_INIT_SCRIPT"

"$_flox_activate_tracer" "$activate_d/zdotdir/.zlogin" END
