#!/bin/bash

# env-filter: redact environment variables, apart from those expressly called out
# with (-a|--allow) args or matching prefixes provided with (-p|--allow-prefix).
# In truth this should not be a separate script at all, but rather `env` itself
# should be modified to add these options.

_env="@coreutils@/bin/env"
_getopt="@getopt@/bin/getopt"

# Parse command-line arguments.
OPTIONS="a:p:d"
LONGOPTS="allow:,allow-prefix:,debug"
USAGE="Usage: $0 [(-a|--allow) <var>] \
[(-p|--allow-prefix) <prefix>] \
[(-d|--debug)] [--] <command> [<args>]
  -a, --allow <var>  allow environment variable <var>
  -p, --allow-prefix <prefix>
                     allow variables starting with <prefix>
  -d, --debug        enable debugging
  -h, --help         show this help message"
PARSED=$("$_getopt" --options="$OPTIONS" --longoptions="$LONGOPTS" --name "$0" -- "$@")
if [[ $? -ne 0 ]]; then
  echo "Failed to parse options."
  exit 1
fi

# Use eval to remove quotes and replace them with spaces.
eval set -- "$PARSED"

# Set default values for options.
declare -a _allow_vars=()
declare -a _allow_prefixes=()
declare -i _debug=0
while true; do
  case "$1" in
    -a|--allow)
      shift
      if [ -z "${1:-}" ]; then
        echo "Option --allow requires a variable name as an argument." >&2
        echo "$USAGE" >&2
        exit 1
      fi
      _allow_vars+=("$1")
      shift
      ;;
    -p|--allow-prefix)
      shift
      if [ -z "${1:-}" ]; then
        echo "Option --allow-prefix requires a prefix string as an argument." >&2
        echo "$USAGE" >&2
        exit 1
      fi
      _allow_prefixes+=("$1")
      shift
      ;;
    -d|--debug)
      let _debug++
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

# Throw error if no command is provided.
if [ $# -eq 0 ]; then
  echo "ERROR: no command provided." >&2
  echo "$USAGE" >&2
  exit 1
fi

# Clear the environment of all variables _not_ specified with -a|--allow.
declare -i _allow
while IFS='=' read -r -d '' n v; do
  _allow=0
  # Keep env vars starting with an allowed prefix.
  for prefix in "${_allow_prefixes[@]}"; do
    if [[ "$n" == "$prefix"* ]]; then
      let _allow++
      continue
    fi
  done
  # Keep env vars that were specifically allowed.
  case " ${_allow_vars[*]} " in
  *" $n "*) let _allow++;;
  esac
  # Discard env vars not specifically allowed.
  if [ $_allow -gt 0 ]; then
    if [ $_debug -gt 0 ]; then
      echo "[DEBUG]  allow  $n=$v" >&2
    fi
  else
    if [ $_debug -gt 0 ]; then
      echo "[DEBUG] discard $n=$v" >&2
    fi
    unset "$n"
  fi
done < <("$_env" -0)

# exec the supplied command
exec "$@"
