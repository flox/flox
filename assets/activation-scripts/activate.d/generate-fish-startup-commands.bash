# shellcheck disable=SC2154

_sed="@gnused@/bin/sed"

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
  FLOX_ENV="${1?}"
  shift
  _FLOX_ACTIVATION_PROFILE_ONLY="${1?}"
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

    # Propagate $_activate_d to the environment.
    echo "set -gx _activate_d $_activate_d;"
    # Propagate $_flox_activate_tracer to the environment.
    echo "set -gx _flox_activate_tracer $_flox_activate_tracer;"
  fi

  # Set the prompt if we're in an interactive shell.
  echo "if isatty 1; source '$_activate_d/set-prompt.fish'; end;"

  # We already customized the PATH and MANPATH, but the user and system
  # dotfiles may have changed them, so finish by doing this again.
  echo "$_flox_env_helper fish | source;"

  # Source user-specified profile scripts if they exist.
  for i in profile-common profile-fish hook-script; do
    if [ -e "$FLOX_ENV/activate.d/$i" ]; then
      "$_flox_activate_tracer" "$FLOX_ENV/activate.d/$i" START
      echo "source '$FLOX_ENV/activate.d/$i';"
      "$_flox_activate_tracer" "$FLOX_ENV/activate.d/$i" END
    else
      "$_flox_activate_tracer" "$FLOX_ENV/activate.d/$i" NOT FOUND
    fi
  done

  # fish does not use hashing in the same way bash does, so there's
  # nothing to be done here by way of that requirement.

  # Disable tracing before potentially launching into user shell.
  if [ "$_flox_activate_tracelevel" -ge 2 ]; then
    echo "set -gx fish_trace 0;"
  fi

  "$_flox_activate_tracer" "generate_fish_startup_commands()" END
}
