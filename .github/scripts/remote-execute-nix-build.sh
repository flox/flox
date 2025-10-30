#!/usr/bin/env bash

set -euxo pipefail

function render_remote_cmd() {
  local -r flake_ref="github:flox/flox/$GITHUB_SHA#$1"
  shift

  local -r nix_build_args=(
    nix build
    --extra-experimental-features '"nix-command flakes"'
    --no-link
    --print-build-logs
    --json
    --accept-flake-config
    "$flake_ref")

  # Don't actually run the command, just render it. We want the environment
  # variables from this machine, not the remote builder.
  echo "
  set -euxo pipefail
  DRV=\"\$(${nix_build_args[@]} | jq -r '.[0].drvPath')\"
  "
}
export -f render_remote_cmd

function main() {
  local -r usage="USAGE: remote-execute-nix-build.sh <attr_path> <remote store>"
  local -r attr_path=${1?$usage}
  shift

  # Execute the render_remote_cmd on the remote host, whilst also keeping a copy
  # of stdout on this machine.
  ssh "github@$REMOTE_SERVER_ADDRESS" \
    -o LogLevel=ERROR \
    -o "UserKnownHostsFile=$REMOTE_SERVER_USER_KNOWN_HOSTS_FILE" \
    "bash -s" \
    < <(render_remote_cmd "$attr_path" "$remote_store")
}
main "$@"
