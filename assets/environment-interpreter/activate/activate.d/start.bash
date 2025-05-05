_comm="@coreutils@/bin/comm"
_daemonize="@daemonize@/bin/daemonize"
_flox_activations="@flox_activations@"
_sed="@gnused@/bin/sed"
_sort="@coreutils@/bin/sort"

start() {
  _flox_activation_state_dir="${1?}"
  shift

  "$_flox_activate_tracer" "$_activate_d/start.bash" START

  # If interactive and a command has not been passed, this is an interactive
  # activate,
  # and we print a message to the user
  # If inside a container, FLOX_ENV_DESCRIPTION won't be set, and we don't need to
  # print a message
  if [ -t 1 ] && [ $# -eq 0 ] && [ -n "${FLOX_ENV_DESCRIPTION:-}" ]; then
    echo "✅ You are now using the environment '$FLOX_ENV_DESCRIPTION'." >&2
    echo "To stop using this environment, type 'exit'" >&2
    echo >&2
  fi

  # First activation of this environment. Snapshot environment to start.
  # Skip if passed no state dir.
  if [ -n "$_flox_activation_state_dir" ]; then
    _start_env="$_flox_activation_state_dir/bare.env"
    export | LC_ALL=C $_sort > "$_start_env"
  fi

  # Process the flox environment customizations, which includes (amongst
  # other things) prepending this environment's bin directory to the PATH.
  # shellcheck disable=SC2154 # set in the main `activate` script
  if [ "$_FLOX_ENV_ACTIVATION_MODE" = "dev" ]; then
    # shellcheck disable=SC1090 # from rendered environment
    source_profile_d "$_profile_d"
  fi

  # Capture post-etc-profiles.env.
  # This is currently unused but could be useful for runtime only environment in
  # the future.
  # Skip if passed no state dir.
  if [ -n "$_flox_activation_state_dir" ]; then
    export | LC_ALL=C $_sort > "$_flox_activation_state_dir/post-etc-profiles.env"
  fi

  # Set static environment variables from the manifest.
  if [ -f "$FLOX_ENV/activate.d/envrc" ]; then
    # shellcheck disable=SC1091 # from rendered environment
    source "$FLOX_ENV/activate.d/envrc"
  fi

  # Start the watchdog if passed a watchdog binary and state dir is not empty
  #
  # hook.on-activate could call `exit`, can leave the activation in a non-ready state
  # It runs in the same shell, and the activation is set to 'ready'
  # only _after_ the hook is run.
  # Start a watchdog to ensure the activation is cleaned up when the process dies.
  if [ -n "${_FLOX_WATCHDOG_BIN:-}" ] && [ -n "$_flox_activation_state_dir" ]; then
    # TODO: Some of these args can be removed after https://github.com/flox/flox/issues/2206
    "$_daemonize" \
      -E _FLOX_WATCHDOG_LOG_LEVEL="${_FLOX_WATCHDOG_LOG_LEVEL:-debug}" \
      "$_FLOX_WATCHDOG_BIN" \
      ${FLOX_DISABLE_METRICS:+--disable-metrics} \
      --log-dir "$_FLOX_ENV_LOG_DIR" \
      --socket "$_FLOX_SERVICES_SOCKET" \
      --flox-env "$FLOX_ENV" \
      --activation-id "$_FLOX_ACTIVATION_ID" \
      --runtime-dir "$FLOX_RUNTIME_DIR"
  fi

  # Source the hook-on-activate script if it exists.
  if [ -e "$FLOX_ENV/activate.d/hook-on-activate" ]; then
    # Nothing good can come from output printed to stdout in the
    # user-provided hook scripts because these can get interpreted
    # as configuration statements by the "in-place" activation
    # mode. So, we'll redirect stdout to stderr.
    set +euo pipefail
    "$_flox_activate_tracer" "$FLOX_ENV/activate.d/hook-on-activate" START
    # shellcheck disable=SC1091 # from rendered environment
    source "$FLOX_ENV/activate.d/hook-on-activate" 1>&2
    "$_flox_activate_tracer" "$FLOX_ENV/activate.d/hook-on-activate" END
    set -euo pipefail
  else
    "$_flox_activate_tracer" "$FLOX_ENV/activate.d/hook-on-activate" NOT FOUND
  fi

  # Capture _end_env and generate _add_env and _del_env.
  # Mark the environment as ready to use for attachments.
  # Skip if passed no state dir.
  if [ -n "$_flox_activation_state_dir" ]; then
    # Capture ending environment.
    _end_env="$_flox_activation_state_dir/post-hook.env"
    export | LC_ALL=C $_sort > "$_end_env"

    # The userShell initialization scripts that follow have the potential to undo
    # the environment modifications performed above, so we must first calculate
    # all changes made to the environment so far so that we can restore them after
    # the userShell initialization scripts have run. We use the `comm(1)` command
    # to compare the starting and ending environment captures (think of it as a
    # better diff for comparing sorted files), and `sed(1)` to format the output
    # in the best format for use in each language-specific activation script.
    _add_env="$_flox_activation_state_dir/add.env"
    _del_env="$_flox_activation_state_dir/del.env"

    # Capture environment variables to _set_ as "key=value" pairs.
    # comm -13: only env declarations unique to `$_end_env` (new declarations)
    LC_ALL=C $_comm -13 "$_start_env" "$_end_env" \
      | $_sed -e 's/^declare -x //' > "$_add_env"

    # Capture environment variables to _unset_ as a list of keys.
    # TODO: remove from $_del_env keys set in $_add_env
    LC_ALL=C $_comm -23 "$_start_env" "$_end_env" \
      | $_sed -e 's/^declare -x //' -e 's/=.*//' > "$_del_env"

    # Finally mark the environment as ready to use for attachments.
    "$_flox_activations" \
      set-ready \
      --runtime-dir "$FLOX_RUNTIME_DIR" \
      --flox-env "$FLOX_ENV" --id "$_FLOX_ACTIVATION_ID"
  fi

  "$_flox_activate_tracer" "$_activate_d/start.bash" END
}
