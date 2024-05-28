#!/usr/bin/env bash

# Call this script with TESTING_FLOX_CATALOG_URL set and a pkg-path for show.
# It will generate a JSON response file produced by running
# `flox show <pkg-path>`
# and save it in a file `<pkg-path>.json`.
# If the dump file already exists, it will be deleted.

set -euo pipefail

if [ -z "${TESTING_FLOX_CATALOG_URL:-}" ]; then
  echo "TESTING_FLOX_CATALOG_URL is not set"
  exit 1
fi
export FLOX_CATALOG_URL="$TESTING_FLOX_CATALOG_URL"

if [ "$#" -lt 1 ]; then
  echo "Usage: $0 <pkg-path>"
  exit 1
fi

pkg_path="$1"
filename="$pkg_path.json"

rm -f "$filename"

export FLOX_FEATURES_USE_CATALOG=true
export _FLOX_CATALOG_DUMP_RESPONSE_FILE="$PWD/$filename"

flox show "$pkg_path"
