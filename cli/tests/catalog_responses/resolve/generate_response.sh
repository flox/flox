#!/usr/bin/env bash

# Call this script with TESTING_FLOX_CATALOG_URL set and a list of packages.
# It will generate a JSON response file produced by running
# `flox install -i pkg1_install_id pkg1 -i pkg2_install_id pkg2 ...`
# and save it in a file `pkg1_pkg2.jsoon`.

set -euo pipefail

if [ -z "${TESTING_FLOX_CATALOG_URL:-}" ]; then
  echo "TESTING_FLOX_CATALOG_URL is not set"
  exit 1
fi
export FLOX_CATALOG_URL="$TESTING_FLOX_CATALOG_URL"

if [ "$#" -lt 1 ]; then
  echo "Usage: $0 <pkg1> [pkg2] ..."
  exit 1
fi

# generate install command and filename
first_pkg="$1"

install_args=(-i "${first_pkg}_install_id" "$first_pkg")
filename="$first_pkg"

for pkg in "${@:2}"; do
  install_args+=(-i "${pkg}_install_id" "$pkg")
  filename+="_$pkg"
done

filename+=".json"

# dump
export FLOX_FEATURES_USE_CATALOG=true
export _FLOX_CATALOG_DUMP_RESPONSE_FILE="$PWD/$filename"
mkdir tmp
pushd tmp
flox init

if flox install -v "${install_args[@]}"; then
  rc=0
else
  rc="$?"
fi

flox delete -f -d .
popd
rm -r tmp
exit "$rc"

