#!/usr/bin/env bash

# Call this script with TESTING_FLOX_CATALOG_URL set.
# To regenerate all responses, pass `true` as an argument.
# It will generate a JSON response for each package listed in
# responses_to_generate.json by running
# `flox install <args from responses_to_generate.json>`
# and save it in a file identified by responses_to_generate.json`.
# If the dump file already exists, it will be deleted.

set -euo pipefail

if [ -z "${TESTING_FLOX_CATALOG_URL:-}" ]; then
  echo "TESTING_FLOX_CATALOG_URL is not set"
  exit 1
fi
export FLOX_CATALOG_URL="$TESTING_FLOX_CATALOG_URL"

if [ "$#" -gt 1 ]; then
  echo "Usage: $0 [true]"
  exit 1
fi

regenerate="${1:-false}"

export FLOX_FEATURES_USE_CATALOG=true

jq -r 'to_entries[] | .key, .value' responses_to_generate.json |
while read -r filename; read -a args; do
  filename="$filename.json"
  if [ "$regenerate" != "true" ] && [ -f "$filename" ]; then
    continue
  fi

  export _FLOX_CATALOG_DUMP_RESPONSE_FILE="$PWD/$filename"
  rm -f -- "$PWD/$filename"

  mkdir tmp
  pushd tmp
  flox init

  # TODO: should be able to drop this once we default to all systems
  flox list -c |
    tomlq --toml-output '.options.systems = ["aarch64-darwin", "x86_64-darwin", "aarch64-linux", "x86_64-linux"]' |
    flox edit -f -

  if flox install -vvv "${args[@]}"; then
    rc=0
  else
    rc="$?"
  fi

  flox delete -f -d .
  popd
  rm -r tmp
  if [ "$rc" -ne 0 ]; then
    echo "Failed to generate $filename"
    exit 1
  fi
done
