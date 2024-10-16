# If interactive and a command has not been passed, this is an interactive
# activate,
# and we print a message to the user
# TODO: should this be printed after scripts?
# Should it be in Rust using message::updated?
if [ -t 1 ] && [ $# -eq 0 ]; then
  echo "✅ You are now using the environment $FLOX_ENV_DESCRIPTION." >&2
  echo "To stop using this environment, type 'exit'" >&2
  echo >&2
fi

if [ $flox_env_found -eq 0 ]; then
  # First activation of this environment. Snapshot environment to start.
  _start_env="$($_coreutils/bin/mktemp --suffix=.start-env)"
  export | $_coreutils/bin/sort > "$_start_env"
else # we know "$_FLOX_ACTIVATE_FORCE_REACTIVATE" == true
  # TODO: restore _start_env
  :
fi

# Capture PID of this "first" activation. This provides the unique
# identifier with which to refer to environment variables associated
# with this environment activation.
FLOX_ENV_PID="$$"

# Set environment variables which represent the cumulative layering
# of flox environments. For the most part this involves prepending
# to the existing variables of the same name.
# TODO: reconcile with CLI which should be setting these. Setting
#       "*_activate" variables to indicate the ones we've seen and
#       processed on the activate script side, and ultimately also
#       for testing/comparison against the CLI-maintained equivalents.
FLOX_ENV_DIRS_activate="$FLOX_ENV${FLOX_ENV_DIRS_activate:+:$FLOX_ENV_DIRS_activate}"
FLOX_ENV_LIB_DIRS_activate="$FLOX_ENV/lib${FLOX_ENV_LIB_DIRS_activate:+:$FLOX_ENV_LIB_DIRS_activate}"
FLOX_PROMPT_ENVIRONMENTS_activate="$FLOX_ENV_DESCRIPTION${FLOX_PROMPT_ENVIRONMENTS_activate:+ $FLOX_PROMPT_ENVIRONMENTS_activate}"
export FLOX_ENV_DIRS_activate FLOX_ENV_LIB_DIRS_activate FLOX_PROMPT_ENVIRONMENTS_activate

# Process the flox environment customizations, which includes (amongst
# other things) prepending this environment's bin directory to the PATH.
if [ -d "$FLOX_ENV/etc/profile.d" ]; then
  declare -a _profile_scripts
  read -r -d '' -a _profile_scripts < <(
    cd "$FLOX_ENV/etc/profile.d" || exit
    shopt -s nullglob
    echo *.sh
  )
  for profile_script in "${_profile_scripts[@]}"; do
    # shellcheck disable=SC1090 # from rendered environment
    source "$FLOX_ENV/etc/profile.d/$profile_script"
  done
  unset _profile_scripts
fi

# Set static environment variables from the manifest.
if [ -f "$FLOX_ENV/activate.d/envrc" ]; then
  # shellcheck disable=SC1091 # from rendered environment
  source "$FLOX_ENV/activate.d/envrc"
fi

# Source the hook-on-activate script if it exists.
if [ -e "$FLOX_ENV/activate.d/hook-on-activate" ]; then
  # Nothing good can come from output printed to stdout in the
  # user-provided hook scripts because these can get interpreted
  # as configuration statements by the "in-place" activation
  # mode. So, we'll redirect stdout to stderr.
  # shellcheck disable=SC1091 # from rendered environment
  source "$FLOX_ENV/activate.d/hook-on-activate" 1>&2
fi

# We only need to capture the ending environment when
# "$_FLOX_ACTIVATE_FORCE_REACTIVATE" isn't set
if [ $flox_env_found -eq 0 ]; then
  # Capture ending environment.
  _end_env="$($_coreutils/bin/mktemp --suffix=.$FLOX_ENV_PID.end-env)"
  export | $_coreutils/bin/sort > "$_end_env"

  # The userShell initialization scripts that follow have the potential to undo
  # the environment modifications performed above, so we must first calculate
  # all changes made to the environment so far so that we can restore them after
  # the userShell initialization scripts have run. We use the `comm(1)` command
  # to compare the starting and ending environment captures (think of it as a
  # better diff for comparing sorted files), and `sed(1)` to format the output
  # in the best format for use in each language-specific activation script.
  _add_env="$($_coreutils/bin/mktemp --suffix=.$FLOX_ENV_PID.add-env)"
  _del_env="$($_coreutils/bin/mktemp --suffix=.$FLOX_ENV_PID.del-env)"

  # Export tempfile paths for use within shell-specific activation scripts.
  export _add_env _del_env

  # Capture environment variables to _set_ as "key=value" pairs.
  # comm -13: only env declarations unique to `$_end_env` (new declarations)
  $_coreutils/bin/comm -13 "$_start_env" "$_end_env" \
    | $_gnused/bin/sed -e 's/^declare -x //' > "$_add_env"

  # Capture environment variables to _unset_ as a list of keys.
  # TODO: remove from $_del_env keys set in $_add_env
  $_coreutils/bin/comm -23 "$_start_env" "$_end_env" \
    | $_gnused/bin/sed -e 's/^declare -x //' -e 's/=.*//' > "$_del_env"

  # Don't need these anymore.
  $_coreutils/bin/rm -f "$_start_env" "$_end_env"
fi
