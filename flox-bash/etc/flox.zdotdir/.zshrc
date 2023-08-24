zshrc=${FLOX_ORIG_ZDOTDIR:-$HOME}/.zshrc

# This is the only file in which we need to perform flox actions so
# take this opportunity to restore the user's original $ZDOTDIR if
# defined, otherwise remove it from the environment.
if [ -n "$FLOX_ORIG_ZDOTDIR" ]
then
	export ZDOTDIR=$FLOX_ORIG_ZDOTDIR
	unset FLOX_ORIG_ZDOTDIR
else
	unset ZDOTDIR
fi

# Do all of the usual initializations.
if [ -f ${zshrc} ]
then
    source ${zshrc}
fi

# Bring in the Nix and Flox environment customizations.
[ -z "$FLOX_ZSH_INIT_SCRIPT" ] || source $FLOX_ZSH_INIT_SCRIPT
