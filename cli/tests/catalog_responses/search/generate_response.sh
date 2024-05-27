#!/usr/bin/env bash

# Call this script with TESTING_FLOX_CATALOG_URL set and args for search
# It will generate a JSON response file produced by running
# `flox search args`
# and save it in a file `underscore_joined_args.json`.
# If the dump file already exists, it will be deleted.

set -euo pipefail

if [ -z "${TESTING_FLOX_CATALOG_URL:-}" ]; then
  echo "TESTING_FLOX_CATALOG_URL is not set"
  exit 1
fi
export FLOX_CATALOG_URL="$TESTING_FLOX_CATALOG_URL"

if [ "$#" -lt 1 ]; then
  echo "Usage: $0 <args for search> ..."
  exit 1
fi

filename="$1"
for arg in "${@:2}"; do
  filename+="_$arg"
done
filename+=".json"

rm -f "$filename"

export FLOX_FEATURES_USE_CATALOG=true
export _FLOX_CATALOG_DUMP_RESPONSE_FILE="$PWD/$filename"

flox search "$@"
