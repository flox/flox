# shellcheck disable=SC2154

_sed="@gnused@/bin/sed"
_flox_activations="@flox_activations@"

# N.B. the output of
# these scripts may be eval'd with backticks which have the effect of removing
# newlines from the output, so we must ensure that the output is a valid shell
# script fragment when represented on a single line.
generate_fish_startup_commands() {
  "$_flox_activate_tracer" "generate_fish_startup_commands()" START

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
    echo "set -gx fish_trace 1;"
  fi

  if [ "${_FLOX_ACTIVATION_PROFILE_ONLY:-}" != true ]; then
    # The fish --init-command option allows us to source our startup
    # file after the normal configuration has been processed, so there
    # is no requirement to go back and source the user's own config
    # as we do in bash.

    # Restore environment variables set in the previous bash initialization.
    $_sed -e 's/^/set -e /' -e 's/$/;/' "$_FLOX_ACTIVATION_STATE_DIR/del.env"
    $_sed -e 's/^/set -gx /' -e 's/=/ /' -e 's/$/;/' "$_FLOX_ACTIVATION_STATE_DIR/add.env"

    # Propagate required variables that are documented as exposed.
    echo "set -gx FLOX_ENV '$_FLOX_ENV';"

    # Propagate optional variables that are documented as exposed.
    for var_key in FLOX_ENV_CACHE FLOX_ENV_PROJECT FLOX_ENV_DESCRIPTION; do
      eval "var_val=\${_$var_key-}"
      if [ -n "$var_val" ]; then
        echo "set -gx $var_key '$var_val';"
      else
        echo "set -e $var_key;"
      fi
    done
  fi

  # Propagate $_activate_d to the environment.
  echo "set -gx _activate_d '$_activate_d';"
  # Propagate $_flox_activate_tracer to the environment.
  echo "set -gx _flox_activate_tracer '$_flox_activate_tracer';"
  # Propagate $_flox_activations to the environment
  echo "set -gx _flox_activations '$_flox_activations';"

  # Set the prompt if we're in an interactive shell.
  echo "if isatty 1; source '$_activate_d/set-prompt.fish'; end;"

  # We already customized the PATH and MANPATH, but the user and system
  # dotfiles may have changed them, so finish by doing this again.

  # fish doesn't have {foo:-} syntax, so we need to provide a temporary variable
  # (foo_with_default) that is either the runtime (not generation-time) value
  # or the string 'empty'.
  echo "set -gx FLOX_ENV_DIRS (if set -q FLOX_ENV_DIRS; echo \"\$FLOX_ENV_DIRS\"; else; echo empty; end);"
  echo "$_flox_activations set-env-dirs --shell fish --flox-env \"$_FLOX_ENV\" --env-dirs \"\$FLOX_ENV_DIRS\" | source;"
  echo "set -gx MANPATH (if set -q MANPATH; echo \"\$MANPATH\"; else; echo empty; end);"
  echo "$_flox_activations fix-paths --shell fish --env-dirs \"\$FLOX_ENV_DIRS\" --path \"\$PATH\" --manpath \"\$MANPATH\" | source;"
  # Source library of shell-specific functions prior to calling the
  # `flox-activations profile-scripts` command which depends on the
  # `source_once()` function.
  echo "source '$_activate_d/functions.fish';"
  echo "$_flox_activations profile-scripts --shell fish --env-dirs \"\$FLOX_ENV_DIRS\" | source;"

  # fish does not use hashing in the same way bash does, so there's
  # nothing to be done here by way of that requirement.

  # Disable tracing before potentially launching into user shell.
  if [ "$_flox_activate_tracelevel" -ge 2 ]; then
    echo "set -gx fish_trace 0;"
  fi

  "$_flox_activate_tracer" "generate_fish_startup_commands()" END
}
