# "Reactivation" of this environment.

# Assert that the expected _{add,del}_env variables are present.
if [ -z "$_add_env" ] || [ -z "$_del_env" ]; then
  echo "ERROR (activate): \$_add_env and \$_del_env not found in environment" >&2
  if [ -h "$FLOX_ENV" ]; then
    echo "moving $FLOX_ENV link to $FLOX_ENV.$$ - please try again" >&2
    $_coreutils/bin/mv "$FLOX_ENV" "$FLOX_ENV.$$"
  fi
  exit 1
fi

# Replay the environment for the benefit of this shell.
eval "$($_gnused/bin/sed -e 's/^/unset /' -e 's/$/;/' "$_del_env")"
eval "$($_gnused/bin/sed -e 's/^/export /' -e 's/$/;/' "$_add_env")"
