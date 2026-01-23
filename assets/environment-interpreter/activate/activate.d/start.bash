#!/usr/bin/env bash
# shellcheck shell=bash

[ "${_flox_activate_tracelevel:?}" -eq 0 ] || set -x

_getopt="@getopt@/bin/getopt"
_jq="@jq@/bin/jq"

_profile_d="__OUT__/etc/profile.d"

set -euo pipefail

# Set umask to ensure files are created with 0600 (may contain secrets)
umask 077

# shellcheck source-path=SCRIPTDIR/activate.d
source "${_activate_d}/helpers.bash"

"$_flox_activate_tracer" "$_activate_d/start.bash" START

# Parse command-line arguments.
OPTIONS="c:m:"
LONGOPTS="command:,\
shell:,\
env-cache:,\
env-project:,\
env-description:,\
mode:,\
start-state-dir:"
USAGE="Usage: $0 [-c \"<cmd> <args>\"] \
[--shell <shell>] \
[--env-cache <path>] \
[--env-project <path>] \
[--env-description <name>] \
[(-m|--mode) (dev|run)] \
[--start-state-dir <path>]"

if ! PARSED=$("$_getopt" --options="$OPTIONS" --longoptions="$LONGOPTS" --name "$0" -- "$@"); then
  echo "Failed to parse options." >&2
  echo "$USAGE" >&2
  exit 1
fi

# Use eval to remove quotes and replace them with spaces.
eval set -- "$PARSED"

# Set default values for options.
FLOX_CMD=""
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
    --start-state-dir)
      shift
      if [ -z "${1:-}" ]; then
        echo "Option --start-state-dir requires a path as an argument." >&2
        echo "$USAGE" >&2
        exit 1
      fi
      _start_state_dir="$1"
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

# First activation of this environment. Snapshot environment to start.
_start_env_json="$_start_state_dir/start.env.json"
$_jq -nS env > "$_start_env_json"

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

# Capture ending environment.
_end_env_json="$_start_state_dir/end.env.json"
$_jq -nS env > "$_end_env_json"

"$_flox_activate_tracer" "$_activate_d/start.bash" END
