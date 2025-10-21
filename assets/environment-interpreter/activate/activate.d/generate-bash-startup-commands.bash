# shellcheck shell=bash
# shellcheck disable=SC2154

_sed="@gnused@/bin/sed"
_flox_activations="@flox_activations@"

# N.B. the output of
# these scripts may be eval'd with backticks which have the effect of removing
# newlines from the output, so we must ensure that the output is a valid shell
# script fragment when represented on a single line.
generate_bash_startup_commands() {
  "$_flox_activate_tracer" "generate_bash_startup_commands()" START

  _flox_activate_tracelevel="${1?}"
  shift
  _FLOX_ACTIVATION_STATE_DIR="${1?}"
  shift
  _activate_d="${1?}"
  shift
  _FLOX_ENV="${1?}"
  shift
  _FLOX_ENV_CACHE="${1?}"
  shift
  _FLOX_ENV_PROJECT="${1?}"
  shift
  _FLOX_ENV_DESCRIPTION="${1?}"
  shift
  _is_in_place="${1?}"
  shift

  if [ "$_flox_activate_tracelevel" -ge 2 ]; then
    echo "set -x;"
  fi

  # We need to source the .bashrc file exactly once. We skip it for in-place
  # activations under the assumption that it has already been sourced by one
  # of the shells in the chain of ancestors UNLESS none of them were bash
  # and therefore .bashrc hasn't been sourced yet.
  # declare needs_sourcing
  # if bashrc exists:
  should_source="false"
  if [ -f ~/.bashrc ] && [ "${_is_in_place:-}" != "true" ] && [ "${_flox_sourcing_rc:-}" != "true" ]; then
    should_source="true"
  fi
  if [ "$should_source" = "true" ]; then
    echo "export _flox_sourcing_rc=true;"
    echo "source ~/.bashrc;"
    echo "unset _flox_sourcing_rc;"
  fi

  # Restore environment variables set in the previous bash initialization.
  $_sed -e 's/^/unset /' -e 's/$/;/' "$_FLOX_ACTIVATION_STATE_DIR/del.env"
  $_sed -e 's/^/export /' -e 's/$/;/' "$_FLOX_ACTIVATION_STATE_DIR/add.env"

  # Propagate required variables that are documented as exposed.
  echo "export FLOX_ENV='$_FLOX_ENV';"

  # Propagate optional variables that are documented as exposed.
  for var_key in FLOX_ENV_CACHE FLOX_ENV_PROJECT FLOX_ENV_DESCRIPTION; do
    eval "var_val=\${_$var_key-}"
    if [ -n "$var_val" ]; then
      echo "export $var_key='$var_val';"
    else
      echo "unset $var_key;"
    fi
  done

  # Propagate $_activate_d to the environment.
  echo "export _activate_d='$_activate_d';"
  # Propagate $_flox_activate_tracer to the environment.
  echo "export _flox_activate_tracer='$_flox_activate_tracer';"
  # Propagate $_flox_activations to the environment
  echo "export _flox_activations='$_flox_activations';"

  # Set the prompt if we're in an interactive shell.
  echo "if [ -t 1 ]; then source '$_activate_d/set-prompt.bash'; fi;"

  # We already customized the PATH and MANPATH, but the user and system
  # dotfiles may have changed them, so finish by doing this again.
  # shellcheck disable=SC1090
  # Use generation time _FLOX_ENV because we want to guarantee we activate the
  # environment we think we're activating. Use runtime FLOX_ENV_DIRS to allow
  # RC files to perform activations.
  echo "eval \"\$('$_flox_activations' set-env-dirs --shell bash --flox-env \"$_FLOX_ENV\" --env-dirs \"\${FLOX_ENV_DIRS:-}\")\";"
  echo "eval \"\$('$_flox_activations' fix-paths --shell bash --env-dirs \"\$FLOX_ENV_DIRS\" --path \"\$PATH\" --manpath \"\${MANPATH:-}\")\";"
  echo "eval \"\$('$_flox_activations' profile-scripts --shell bash --already-sourced-env-dirs \"\${_FLOX_SOURCED_PROFILE_SCRIPTS:-}\" --env-dirs \"\${FLOX_ENV_DIRS:-}\")\";"

  # Disable command hashing to allow for newly installed flox packages
  # to be found immediately. We do this as the very last thing because
  # python venv activations can otherwise return nonzero return codes
  # when attempting to invoke `hash -r`.
  echo "set +h;"

  if [ "$_flox_activate_tracelevel" -ge 2 ]; then
    echo "set +x;"
  fi

  "$_flox_activate_tracer" "generate_bash_startup_commands()" END
}
