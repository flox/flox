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
  _FLOX_ENV="${1?}"
  shift
  _FLOX_ENV_CACHE="${1?}"
  shift
  _FLOX_ENV_PROJECT="${1?}"
  shift
  _FLOX_ENV_DESCRIPTION="${1?}"
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

    # Propagate required variables that are documented as exposed.
    echo "setenv FLOX_ENV '$_FLOX_ENV';"

    # Propagate optional variables that are documented as exposed.
    for var_key in FLOX_ENV_CACHE FLOX_ENV_PROJECT FLOX_ENV_DESCRIPTION; do
      eval "var_val=\${_$var_key-}"
      if [ -n "$var_val" ]; then
        echo "setenv $var_key '$var_val';"
      else
        echo "unsetenv $var_key;"
      fi
    done
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
  # Use generation time _FLOX_ENV because we want to guarantee we activate the
  # environment we think we're activating. Use runtime FLOX_ENV_DIRS to allow
  # RC files to perform activations.
  echo 'if (! $?FLOX_ENV_DIRS) setenv FLOX_ENV_DIRS "empty";'
  echo "eval \"\`'$_flox_activations' set-env-dirs --shell tcsh --flox-env '$_FLOX_ENV' --env-dirs \$FLOX_ENV_DIRS:q\`\";"
  echo 'if (! $?MANPATH) setenv MANPATH "empty";'
  echo "eval \"\`'$_flox_activations' fix-paths --shell tcsh --env-dirs \$FLOX_ENV_DIRS:q --path \$PATH:q --manpath \$MANPATH:q\`\";"

  # Modern versions of tcsh support the ":Q" modifier for passing empty args
  # on the command line, but versions prior to 6.23 do not have a way to do
  # that, so to support these versions we will instead avoid passing the
  # --already-sourced-env-dirs argument altogether when there is no default
  # value to be passed.
  printf "%s;\n%s;\n%s;\n" \
    "set _already_sourced_args = ()" \
    "if (\$?_FLOX_SOURCED_PROFILE_SCRIPTS) set _already_sourced_args = ( --already-sourced-env-dirs \`echo \$_FLOX_SOURCED_PROFILE_SCRIPTS:q\` )" \
    "eval \"\`'$_flox_activations' profile-scripts --shell tcsh --env-dirs \$FLOX_ENV_DIRS:q \$_already_sourced_args:q\`\""

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
