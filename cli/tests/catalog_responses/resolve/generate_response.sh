#!/usr/bin/env bash

# Call this script with TESTING_FLOX_CATALOG_URL set and a list of packages.
# It will generate a JSON response file produced by running
# `flox install -i pkg1_install_id pkg1 -i pkg2_install_id pkg2 ...`
# and save it in a file `pkg1_pkg2.json`.
# If the dump file already exists, it will be deleted.

set -euo pipefail

if [ -z "${TESTING_FLOX_CATALOG_URL:-}" ]; then
  echo "TESTING_FLOX_CATALOG_URL is not set"
  exit 1
fi
export FLOX_CATALOG_URL="$TESTING_FLOX_CATALOG_URL"

if [ "$#" -lt 1 ]; then
  echo "Usage: $0 <args> [for] [install]..."
  exit 1
fi

filename="$*.json"

rm -f -- "$filename"

# dump
export FLOX_FEATURES_USE_CATALOG=true
export _FLOX_CATALOG_DUMP_RESPONSE_FILE="$PWD/$filename"
mkdir tmp
pushd tmp
flox init

# TODO: should be able to drop this once we default to all systems
flox list -c |
  tomlq --toml-output '.options.systems = ["aarch64-darwin", "x86_64-darwin", "aarch64-linux", "x86_64-linux"]' |
  flox edit -f -

if flox install "$@"; then
  rc=0
else
  rc="$?"
fi

flox delete -f -d .
popd
rm -r tmp
exit "$rc"

