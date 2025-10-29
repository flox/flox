_comm="@coreutils@/bin/comm"
_daemonize="@daemonize@/bin/daemonize"
_flox_activations="@flox_activations@"
_jq="@jq@/bin/jq"
_sed="@gnused@/bin/sed"
_sort="@coreutils@/bin/sort"

# Run activate hook
# If $1 is an empty string, the environment is not captured,
# and the activation is not added to the activation registry.
# If $1 is not empty, it is used to capture the environment changes made by the
# hook.
start() {
  _flox_activation_state_dir="${1?}"
  shift
  _flox_shell_mode="${1?}"
  shift

  if [ -z "$_flox_activation_state_dir" ]; then
    echo "Error: _flox_activation_state_dir cannot be empty" >&2
    exit 1
  fi

  "$_flox_activate_tracer" "$_activate_d/start.bash" START

  # Don't clobber STDERR or recommend 'exit' for non-interactive shells.
  # If inside a container, FLOX_ENV_DESCRIPTION won't be set, and we don't need to
  # print a message
  if [ "${_flox_shell_mode}" = "interactive" ] && [ -n "${FLOX_ENV_DESCRIPTION:-}" ]; then
    echo "✅ You are now using the environment '$FLOX_ENV_DESCRIPTION'." >&2
    echo "To stop using this environment, type 'exit'" >&2
    echo >&2
  fi

  # First activation of this environment. Snapshot environment to start.
  # Use jq to capture environment as properly-escaped JSON
  _start_env="$_flox_activation_state_dir/start.env.json"
  $_jq -nS env > "$_start_env"

  # Process the flox environment customizations, which includes (amongst
  # other things) prepending this environment's bin directory to the PATH.
  # shellcheck disable=SC2154 # set in the main `activate` script
  if [ "$_FLOX_ENV_ACTIVATION_MODE" = "dev" ] || [ "$_FLOX_ENV_ACTIVATION_MODE" = "start" ]; then
    # shellcheck disable=SC1090 # from rendered environment
    source_profile_d "$_profile_d" "prepend" "$FLOX_ENV_DIRS"
  else
    # shellcheck disable=SC1091 # from rendered environment
    source "$_profile_d/0100_common-run-mode-paths.sh"
  fi

  # Capture post-etc-profiles.env.
  # This is currently unused but could be useful for runtime only environment in
  # the future.
  $_jq -nS env > "$_flox_activation_state_dir/post-etc-profiles.env.json"

  # Set static environment variables from the manifest.
  set_manifest_vars "$FLOX_ENV"

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
  # Capture ending environment as JSON.
  _end_env="$_flox_activation_state_dir/end.env.json"
  $_jq -nS env > "$_end_env"

  # The userShell initialization scripts that follow have the potential to undo
  # the environment modifications performed above, so we must first calculate
  # all changes made to the environment so far so that we can restore them after
  # the userShell initialization scripts have run. We use jq to diff the two
  # JSON environment snapshots.
  _add_env="$_flox_activation_state_dir/add.env"
  _del_env="$_flox_activation_state_dir/del.env"

  # Capture environment variables to _set_ as "key=value" pairs with proper JSON escaping.
  # This finds keys that are new or changed in $_end_env compared to $_start_env.
  $_jq -rS --slurpfile start "$_start_env" '
    to_entries |
    map(select(
      ($start[0][.key] // null) != .value
    )) |
    map("\(.key)=\(.value)") |
    .[]
  ' "$_end_env" > "$_add_env"

  # Capture environment variables to _unset_ as a list of keys.
  # This finds keys that exist in $_start_env but not in $_end_env.
  $_jq -rS --slurpfile end "$_end_env" '
    to_entries |
    map(select(
      ($end[0][.key] // null) == null
    )) |
    map(.key) |
    .[]
  ' "$_start_env" > "$_del_env"

  # Finally mark the environment as ready to use for attachments.
  "$_flox_activations" \
    set-ready \
    --runtime-dir "$FLOX_RUNTIME_DIR" \
    --flox-env "$FLOX_ENV" --id "$_FLOX_ACTIVATION_ID"

  "$_flox_activate_tracer" "$_activate_d/start.bash" END
}
