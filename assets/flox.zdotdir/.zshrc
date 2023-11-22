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

source "$flox_zdotdir/prompt.zshrc"

if [ -d "$FLOX_ENV/etc/profile.d" ]; then
  declare -a _prof_scripts;
  _prof_scripts=( $(
    set -o nullglob;
    echo "$FLOX_ENV/etc/profile.d"/*.sh;
  ) );
  for p in "${_prof_scripts[@]}"; do . "$p"; done
  unset _prof_scripts;
fi

# Bring in the Nix and Flox environment customizations.
[ -z "$FLOX_ZSH_INIT_SCRIPT" ] || source "$FLOX_ZSH_INIT_SCRIPT"
