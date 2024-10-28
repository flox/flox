# If interactive and a command has not been passed, this is an interactive
# activate,
# and we print a message to the user
# If inside a container, FLOX_ENV_DESCRIPTION won't be set, and we don't need to
# print a message
if [ -t 1 ] && [ $# -eq 0 ]; then
  echo "✅ You are now using the environment '$FLOX_ENV_DESCRIPTION'." >&2
  echo "To stop using this environment, type 'exit'" >&2
  echo >&2
fi

# First activation of this environment. Snapshot environment to start.
_start_env="$_FLOX_ACTIVATION_STATE_DIR/bare.env"
export | LC_ALL=C $_coreutils/bin/sort > "$_start_env"

# Process the flox environment customizations, which includes (amongst
# other things) prepending this environment's bin directory to the PATH.
if [ -d "$FLOX_ENV/etc/profile.d" ]; then
  declare -a _profile_scripts
  # TODO: figure out why this is needed
  set +e
  read -r -d '' -a _profile_scripts < <(
    cd "$FLOX_ENV/etc/profile.d" || exit
    shopt -s nullglob
    echo *.sh
  )
  set -e
  for profile_script in "${_profile_scripts[@]}"; do
    # shellcheck disable=SC1090 # from rendered environment
    source "$FLOX_ENV/etc/profile.d/$profile_script"
  done
  unset _profile_scripts
fi

# Capture post-etc-profiles.env.
# This is currently unused but could be useful for runtime only environment in
# the future.
export | LC_ALL=C $_coreutils/bin/sort > "$_FLOX_ACTIVATION_STATE_DIR/post-etc-profiles.env"

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
  set +euo pipefail
  # shellcheck disable=SC1091 # from rendered environment
  source "$FLOX_ENV/activate.d/hook-on-activate" 1>&2
  set -euo pipefail
fi

# Capture ending environment.
_end_env="$_FLOX_ACTIVATION_STATE_DIR/post-hook.env"
export | LC_ALL=C $_coreutils/bin/sort > "$_end_env"

# The userShell initialization scripts that follow have the potential to undo
# the environment modifications performed above, so we must first calculate
# all changes made to the environment so far so that we can restore them after
# the userShell initialization scripts have run. We use the `comm(1)` command
# to compare the starting and ending environment captures (think of it as a
# better diff for comparing sorted files), and `sed(1)` to format the output
# in the best format for use in each language-specific activation script.
_add_env="$_FLOX_ACTIVATION_STATE_DIR/add.env"
_del_env="$_FLOX_ACTIVATION_STATE_DIR/del.env"

# Capture environment variables to _set_ as "key=value" pairs.
# comm -13: only env declarations unique to `$_end_env` (new declarations)
LC_ALL=C $_coreutils/bin/comm -13 "$_start_env" "$_end_env" \
  | $_gnused/bin/sed -e 's/^declare -x //' > "$_add_env"

# Capture environment variables to _unset_ as a list of keys.
# TODO: remove from $_del_env keys set in $_add_env
LC_ALL=C $_coreutils/bin/comm -23 "$_start_env" "$_end_env" \
  | $_gnused/bin/sed -e 's/^declare -x //' -e 's/=.*//' > "$_del_env"

# Start the watchdog if invoked by `flox activate` but not when the
# `${FLOX_ENV}/activate` is invoked directly such as:
# - containers
# - wrapped `flox build` binaries.
#
# This must come before sourcing the complete environment (in case the watchdog
# later depends on vars and hooks) but before the activation is marked as ready
# (to ensure that it gets cleaned up).
if [ -n "${_FLOX_WATCHDOG_BIN:-}" ]; then
  # TODO: Some of these args can be removed after https://github.com/flox/flox/issues/2206
  "$_daemonize" \
    -E _FLOX_WATCHDOG_LOG_LEVEL="debug" \
    "$_FLOX_WATCHDOG_BIN" \
    ${FLOX_DISABLE_METRICS:+--disable-metrics} \
    --log-dir "$_FLOX_ENV_LOG_DIR" \
    --socket "$_FLOX_SERVICES_SOCKET" \
    --pid "$$" \
    --registry "$_FLOX_REGISTRY_PATH" \
    --hash "$_FLOX_DOTFLOX_HASH"
fi

# Finally mark the environment as ready to use for attachments.
"$_flox_activations" \
  ${FLOX_RUNTIME_DIR:+--runtime-dir "$FLOX_RUNTIME_DIR"} \
  set-ready \
  --flox-env "$FLOX_ENV" --id "$_FLOX_ACTIVATION_ID"
