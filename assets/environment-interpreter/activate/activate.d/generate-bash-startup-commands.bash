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
  _FLOX_ACTIVATION_PROFILE_ONLY="${1?}"
  shift
  _FLOX_ENV="${1?}"
  shift
  _FLOX_ENV_CACHE="${1?}"
  shift
  _FLOX_ENV_PROJECT="${1?}"
  shift
  _FLOX_ENV_DESCRIPTION="${1?}"
  shift

  if [ "$_flox_activate_tracelevel" -ge 2 ]; then
    echo "set -x;"
  fi

  if [ "${_FLOX_ACTIVATION_PROFILE_ONLY:-}" != true ]; then
    # TODO: should we skip this for in-place activations?
    # We use --rcfile to activate using bash which skips sourcing ~/.bashrc,
    # so source that here, but not if we're already in the process of sourcing
    # bashrc in a parent process.
    if [ -f ~/.bashrc ] && [ -z "${_flox_already_sourcing_bashrc:=}" ]; then
      echo "export _flox_already_sourcing_bashrc=1;"
      echo "source ~/.bashrc;"
      echo "unset _flox_already_sourcing_bashrc;"
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
  fi

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
  echo "source <('$_flox_activations' set-env-dirs --shell bash --flox-env '$_FLOX_ENV' --env-dirs '${FLOX_ENV_DIRS:-}');"
  echo "source <('$_flox_activations' fix-paths --shell bash --env-dirs '$FLOX_ENV_DIRS' --path '$PATH' --manpath '${MANPATH:-}');"

  # Iterate over $FLOX_ENV_DIRS in reverse order and
  # source user-specified profile scripts if they exist.
  local -a _flox_env_dirs
  # The `source ~/.bashrc` above may modify FLOX_ENV_DIRS,
  # and then _flox_env_helper may fix it up.
  # If this happens, we want to respect those modifications,
  # so we use FLOX_ENV_DIRS from the environment
  IFS=':' read -r -a _flox_env_dirs <<< "$FLOX_ENV_DIRS"
  for ((x = ${#_flox_env_dirs[@]} - 1; x >= 0; x--)); do
    local _flox_env="${_flox_env_dirs["$x"]}"
    for i in profile-common profile-bash hook-script; do
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
  echo "set +h;"

  if [ "$_flox_activate_tracelevel" -ge 2 ]; then
    echo "set +x;"
  fi

  "$_flox_activate_tracer" "generate_bash_startup_commands()" END
}
