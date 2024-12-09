_sed="@gnused@/bin/sed"

# N.B. the output of
# these scripts may be eval'd with backticks which have the effect of removing
# newlines from the output, so we must ensure that the output is a valid shell
# script fragment when represented on a single line.
generate_fish_startup_commands() {
  _flox_activate_tracelevel="${1?}"
  shift
  _FLOX_ACTIVATION_STATE_DIR="${1?}"
  shift
  _FLOX_RESTORE_PATH="${1?}"
  shift
  _FLOX_RESTORE_MANPATH="${1?}"
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

    # Restore PATH and MANPATH if set in one of the attach scripts.
    if [ -n "$_FLOX_RESTORE_PATH" ]; then
      echo "set -gx PATH $_FLOX_RESTORE_PATH;"
    fi
    if [ -n "$_FLOX_RESTORE_MANPATH" ]; then
      echo "set -gx MANPATH $_FLOX_RESTORE_MANPATH;"
    fi
  fi

  # Set the prompt if we're in an interactive shell.
  echo "if isatty 1; source '$_activate_d/set-prompt.fish'; end;"

  # Source user-specified profile scripts if they exist.
  for i in profile-common profile-fish hook-script; do
    if [ -e "$FLOX_ENV/activate.d/$i" ]; then
      echo "source '$FLOX_ENV/activate.d/$i';"
    fi
  done

  # fish does not use hashing in the same way bash does, so there's
  # nothing to be done here by way of that requirement.

  # Disable tracing before potentially launching into user shell.
  if [ "$_flox_activate_tracelevel" -ge 2 ]; then
    echo "set -gx fish_trace 0;"
  fi
}
