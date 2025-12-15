#!/usr/bin/env bash
# shellcheck shell=bash

[ "${_flox_activate_tracelevel:?}" -eq 0 ] || set -x

_daemonize="@daemonize@/bin/daemonize"
_getopt="@getopt@/bin/getopt"
_flox_activations="@flox_activations@"
_jq="@jq@/bin/jq"

# TODO remove
_comm="@coreutils@/bin/comm"
_sed="@gnused@/bin/sed"
_sort="@coreutils@/bin/sort"

_profile_d="__OUT__/etc/profile.d"

set -euo pipefail

# shellcheck source-path=SCRIPTDIR/activate.d
source "${_activate_d}/helpers.bash"

"$_flox_activate_tracer" "$_activate_d/start.bash" START

# Parse command-line arguments.
OPTIONS="e:c:m:"
LONGOPTS="command:,\
shell:,\
env-cache:,\
env-project:,\
env-description:,\
mode:,\
watchdog:,\
activation-state-dir:,\
invocation-type:,\
activation-id:,\
noprofile"
USAGE="Usage: $0 [-c \"<cmd> <args>\"] \
[--shell <shell>] \
[--env-cache <path>] \
[--env-project <path>] \
[--env-description <name>] \
[--noprofile] \
[(-m|--mode) (dev|run)] \
[--activation-state-dir <path>] \
[--invocation-type <type>] \
[--activation-id <id>]"

if ! PARSED=$("$_getopt" --options="$OPTIONS" --longoptions="$LONGOPTS" --name "$0" -- "$@"); then
  echo "Failed to parse options." >&2
  echo "$USAGE" >&2
  exit 1
fi

# Use eval to remove quotes and replace them with spaces.
eval set -- "$PARSED"

# Set default values for options.
FLOX_CMD=""
FLOX_NOPROFILE="${FLOX_NOPROFILE:-}"
# The rust CLI contains sophisticated logic to detect the shell based on
# $FLOX_SHELL or the process listening on STDOUT, but that won't happen when
# activating from the top-level activation script, so fall back to $SHELL as a
# default.
_FLOX_SHELL="$SHELL"
_FLOX_ENV_ACTIVATION_MODE="dev"
while true; do
  case "$1" in
    -c | --command)
      shift
      if [ -z "${1:-}" ]; then
        echo "Option -c requires an argument." >&2
        echo "$USAGE" >&2
        exit 1
      fi
      FLOX_CMD="$1"
      shift
      ;;
    --shell)
      shift
      if [ -z "${1:-}" ]; then
        echo "Option --shell requires a command as an argument." >&2
        echo "$USAGE" >&2
        exit 1
      fi
      _FLOX_SHELL="$1"
      shift
      ;;
    --env-cache)
      shift
      if [ -z "${1:-}" ] || [ ! -d "$1" ]; then
        echo "Option --env-cache requires a valid path as an argument." >&2
        echo "$USAGE" >&2
        exit 1
      fi
      _FLOX_ENV_CACHE="$1"
      shift
      ;;
    --env-project)
      shift
      if [ -z "${1:-}" ] || [ ! -d "$1" ]; then
        echo "Option --env-project requires a valid path as an argument." >&2
        echo "$USAGE" >&2
        exit 1
      fi
      _FLOX_ENV_PROJECT="$1"
      shift
      ;;
    --env-description)
      shift
      if [ -z "${1:-}" ]; then
        echo "Option --env-description requires a name as an argument." >&2
        echo "$USAGE" >&2
        exit 1
      fi
      _FLOX_ENV_DESCRIPTION="$1"
      shift
      ;;
    -m | --mode)
      shift
      if [ -z "${1:-}" ] || ! { [ "$1" == "run" ] || [ "$1" == "dev" ]; }; then
        echo "Option --mode requires 'dev' or 'run' as an argument." >&2
        echo "$USAGE" >&2
        exit 1
      fi
      _FLOX_ENV_ACTIVATION_MODE="$1"
      shift
      ;;
    --activation-state-dir)
      shift
      if [ -z "${1:-}" ]; then
        echo "Option --activation-state-dir requires a path as an argument." >&2
        echo "$USAGE" >&2
        exit 1
      fi
      _flox_activation_state_dir="$1"
      shift
      ;;
    --invocation-type)
      shift
      if [ -z "${1:-}" ]; then
        echo "Option --invocation-type requires a type as an argument." >&2
        echo "$USAGE" >&2
        exit 1
      fi
      _flox_invocation_type="$1"
      shift
      ;;
    --activation-id)
      shift
      if [ -z "${1:-}" ]; then
        echo "Option --activation-id requires an id as an argument." >&2
        echo "$USAGE" >&2
        exit 1
      fi
      _FLOX_ACTIVATION_ID="$1"
      shift
      ;;
    --noprofile)
      FLOX_NOPROFILE="true"
      shift
      ;;
    --)
      shift
      break
      ;;
    -*)
      echo "Invalid option: $1" >&2
      echo "$USAGE" >&2
      exit 1
      ;;
  esac
done

# Don't clobber STDERR or recommend 'exit' for non-interactive shells.
# If inside a container, FLOX_ENV_DESCRIPTION won't be set, and we don't need to
# print a message
if [ "${_flox_invocation_type}" = "interactive" ] && [ -n "${FLOX_ENV_DESCRIPTION:-}" ]; then
  echo "✅ You are now using the environment '$FLOX_ENV_DESCRIPTION'." >&2
  echo "To stop using this environment, type 'exit'" >&2
  echo >&2
fi

# First activation of this environment. Snapshot environment to start.
_start_env_json="$_flox_activation_state_dir/start.env.json"
$_jq -nS env > "$_start_env_json"

# TODO remove
_start_env="$_flox_activation_state_dir/bare.env"
export | LC_ALL=C $_sort > "$_start_env"

# Process the flox environment customizations, which includes (amongst
# other things) prepending this environment's bin directory to the PATH.
# shellcheck disable=SC2154 # set in the main `activate` script
if [ "$_FLOX_ENV_ACTIVATION_MODE" = "dev" ]; then
  # shellcheck disable=SC1090 # from rendered environment
  source_profile_d "$_profile_d" "prepend" "$FLOX_ENV_DIRS"
else
  # shellcheck disable=SC1091 # from rendered environment
  source "$_profile_d/0100_common-run-mode-paths.sh"
fi

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
# Capture ending environment.
_end_env_json="$_flox_activation_state_dir/end.env.json"
$_jq -nS env > "$_end_env_json"

# TODO remove
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
# TODO end remove

# Finally mark the environment as ready to use for attachments.
"$_flox_activations" \
  set-ready \
  --runtime-dir "$FLOX_RUNTIME_DIR" \
  --dot-flox-path "$_FLOX_DOT_FLOX_PATH" \
  --id "$_FLOX_ACTIVATION_ID"

"$_flox_activate_tracer" "$_activate_d/start.bash" END
