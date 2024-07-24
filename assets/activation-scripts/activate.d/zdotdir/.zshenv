# Source /etc/zshenv and "${FLOX_ORIG_ZDOTDIR:-$HOME}/.zshenv" if they exist
# prior to performing Flox-specific initialization.
#
# See README.md for more information on the initialization process.

if [ -f /etc/zshenv ]
then
    if [ -n "${FLOX_ORIG_ZDOTDIR:-}" ]
    then
        ZDOTDIR="$FLOX_ORIG_ZDOTDIR" FLOX_ORIG_ZDOTDIR= source /etc/zshenv
    else
        ZDOTDIR= source /etc/zshenv
    fi
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
fi

# Bring in the Nix and Flox environment customizations, but _not_ if this is
# an interactive or login shell. If the shell is either of these then the
# neighbouring .zshrc or .zlogin files will be sourced after this one, and we
# want to delay processing of the flox init script to the last possible moment
# so that no other "rc" files have an opportunity to perturb the environment
# after we've set it up.
[[ -o interactive ]] || [[ -o login ]] || \
  [ -z "$FLOX_ZSH_INIT_SCRIPT" ] || source "$FLOX_ZSH_INIT_SCRIPT"
