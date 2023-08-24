# Do all of the usual initializations.
if [ -f ~/.bashrc ]
then
    source ~/.bashrc
fi

# Bring in the Nix and Flox environment customizations.
[ -z "$FLOX_BASH_INIT_SCRIPT" ] || source $FLOX_BASH_INIT_SCRIPT
