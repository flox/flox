# shellcheck shell=bash
# shellcheck disable=SC2154

_sed="@gnused@/bin/sed"

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
  FLOX_ENV="${1?}"
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
  # Propagate $_flox_env_helper to the environment.
  echo "setenv _flox_env_helper '$_flox_env_helper';"

  # Set the prompt if we're in an interactive shell.
  echo "if ( \$?tty ) then; source '$_activate_d/set-prompt.tcsh'; endif;"

  # We already customized the PATH and MANPATH, but the user and system
  # dotfiles may have changed them, so finish by doing this again.
  echo "eval \"\`$_flox_env_helper tcsh\`\";"

  # Source user-specified profile scripts if they exist.
  for i in profile-common profile-tcsh; do
    if [ -e "$FLOX_ENV/activate.d/$i" ]; then
      "$_flox_activate_tracer" "$FLOX_ENV/activate.d/$i" START
      echo "source '$FLOX_ENV/activate.d/$i';"
      "$_flox_activate_tracer" "$FLOX_ENV/activate.d/$i" END
    else
      "$_flox_activate_tracer" "$FLOX_ENV/activate.d/$i" NOT FOUND
    fi
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
