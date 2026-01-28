_flox_activations="@flox_activations@"

"$_flox_activate_tracer" "$_activate_d/zdotdir/.zprofile" START

# Source /etc/zprofile and "${FLOX_ORIG_ZDOTDIR:-$HOME}/.zprofile" if they exist.
#
# See README.md for more information on the initialization process.

# Save and restore the current tracelevel in the event that sourcing
# bashrc launches an inner nested activation which unsets it.
_save_FLOX_ORIG_ZDOTDIR="$FLOX_ORIG_ZDOTDIR"
_save_ZDOTDIR="$ZDOTDIR"
_save_flox_activate_tracer="$_flox_activate_tracer"
_save_FLOX_ZSH_INIT_SCRIPT="$FLOX_ZSH_INIT_SCRIPT"

restore_saved_vars() {
    unset _flox_sourcing_rc
    export FLOX_ORIG_ZDOTDIR="$_save_FLOX_ORIG_ZDOTDIR"
    export ZDOTDIR="$_save_ZDOTDIR"
    # TODO: I don't think we should export this but it's needed by set-prompt.zsh
    export _flox_activate_tracer="$_save_flox_activate_tracer"
    export FLOX_ZSH_INIT_SCRIPT="$_save_FLOX_ZSH_INIT_SCRIPT"
}

if [ -f /etc/zprofile ]
then
    export _flox_sourcing_rc=1
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
    export _flox_sourcing_rc=1
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
