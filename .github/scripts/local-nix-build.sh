#!/usr/bin/env bash

# Build a package on this runner and publish its closure, the local
# counterpart of remote-execute-nix-build.sh. The build-time closure stays
# in /nix, which the job keeps on a sticky disk.

set -euxo pipefail

function main() {
  local -r usage="USAGE: local-nix-build.sh <attr_path> <remote store>"
  local -r attr_path=${1?$usage}
  shift
  local -r remote_store=${1?$usage}

  local drv
  drv="$(nix build \
    --no-link \
    --print-build-logs \
    --json \
    ".#$attr_path" | jq -r '.[0].drvPath')"

  # Same signing key that common-setup wrote for the CI cache.
  nix copy \
    --to "$remote_store?secret-key=/tmp/secret-key" \
    "$drv" \
    "$drv^*"
}
main "$@"
