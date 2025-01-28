# shellcheck shell=bash
# shellcheck disable=SC2154

_sed="@gnused@/bin/sed"
_flox_activations="@flox_activations@"

# N.B. the output of
# these scripts may be eval'd with backticks which have the effect of removing
# newlines from the output, so we must ensure that the output is a valid shell
# script fragment when represented on a single line.
generate_tcsh_startup_commands() {
  "$_flox_activate_tracer" "generate_tcsh_startup_commands()" START

  _flox_activate_tracelevel="${1?}"
  shift
  _FLOX_ACTIVATION_STATE_DIR="${1?}"
  shift
  _activate_d="${1?}"
  shift
  _FLOX_ACTIVATION_PROFILE_ONLY="${1?}"
  shift

  if [ "$_flox_activate_tracelevel" -ge 2 ]; then
    echo "set verbose;"
  fi

  if [ "${_FLOX_ACTIVATION_PROFILE_ONLY:-}" != true ]; then
    # The tcsh implementation will source our custom .tcshrc
    # which will then source the result of this script as $FLOX_TCSH_INIT_SCRIPT
    # after the normal configuration has been processed,
    # so there is no requirement to go back and source the user's own config
    # as we do in bash.

    # Restore environment variables set in the previous bash initialization.
    $_sed -e 's/^/unsetenv /' -e 's/$/;/' "$_FLOX_ACTIVATION_STATE_DIR/del.env"
    $_sed -e 's/^/setenv /' -e 's/=/ /' -e 's/$/;/' "$_FLOX_ACTIVATION_STATE_DIR/add.env"
  fi

  # Propagate $_activate_d to the environment.
  echo "setenv _activate_d '$_activate_d';"
  # Propagate $_flox_activate_tracer to the environment.
  echo "setenv _flox_activate_tracer '$_flox_activate_tracer';"
  # Propagate $_flox_activations to the environment
  echo "setenv _flox_activations '$_flox_activations';"

  # Set the prompt if we're in an interactive shell.
  echo "if ( \$?tty ) then; source '$_activate_d/set-prompt.tcsh'; endif;"

  # We already customized the PATH and MANPATH, but the user and system
  # dotfiles may have changed them, so finish by doing this again.
  echo "eval \"\`'$_flox_activations' set-env-dirs --shell tcsh --flox-env '$FLOX_ENV' --env-dirs '${FLOX_ENV_DIRS:-}'\`\";"
  echo "eval \"\`'$_flox_activations' fix-paths --shell tcsh --env-dirs '$FLOX_ENV_DIRS' --path '$PATH' --manpath '${MANPATH:-}'\`\";"

  # Iterate over $FLOX_ENV_DIRS in reverse order and
  # source user-specified profile scripts if they exist.
  # Our custom .tcshrc sources users files that may modify FLOX_ENV_DIRS,
  # and then _flox_env_helper may fix it up.
  # If this happens, we want to respect those modifications,
  # so we use FLOX_ENV_DIRS from the environment
  local -a _flox_env_dirs
  IFS=':' read -r -a _flox_env_dirs <<< "$FLOX_ENV_DIRS"
  for ((x = ${#_flox_env_dirs[@]} - 1; x >= 0; x--)); do
    local _flox_env="${_flox_env_dirs["$x"]}"
    for i in profile-common profile-tcsh; do
      if [ -e "$_flox_env/activate.d/$i" ]; then
        "$_flox_activate_tracer" "$_flox_env/activate.d/$i" START
        echo "source '$_flox_env/activate.d/$i';"
        "$_flox_activate_tracer" "$_flox_env/activate.d/$i" END
      else
        "$_flox_activate_tracer" "$_flox_env/activate.d/$i" NOT FOUND
      fi
    done
  done

  # Disable command hashing to allow for newly installed flox packages
  # to be found immediately. We do this as the very last thing because
  # python venv activations can otherwise return nonzero return codes
  # when attempting to invoke `hash -r`.
  echo "unhash;"

  if [ "$_flox_activate_tracelevel" -ge 2 ]; then
    echo "unset verbose;"
  fi

  "$_flox_activate_tracer" "generate_tcsh_startup_commands()" END
}
