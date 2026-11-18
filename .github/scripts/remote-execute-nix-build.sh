#!/usr/bin/env bash

set -euxo pipefail

function render_remote_cmd() {
  local -r flake_ref="github:flox/flox/$GITHUB_SHA#$1"
  shift
  local -r remote_store=$1
  shift
  # Whether to publish the build-time outputs as well, see below.
  local -r copy_build_outputs=${1:-false}

  local -r nix_build_args=(
    nix build
    --extra-experimental-features '"nix-command flakes"'
    --no-link
    --print-build-logs
    --json
    --accept-flake-config
    "$flake_ref")

  local -r nix_copy_cmd=(
    nix copy
    --extra-experimental-features nix-command
    --substitute-on-destination
    --to $remote_store?secret-key=/tmp/nix-substituter-secret-key
  )

  local -r nix_copy_args=(
    "${nix_copy_cmd[@]}"
    \$DRV
    \$DRV^*
  )

  # A crate's build artifact is a build input of the next crate, never a
  # runtime reference of the binary, so copying the outputs above does not
  # reach it. Publishing those lets a builder with a cold store substitute the
  # dependency graph instead of recompiling it.
  #
  # Restricted to the crate artifacts: the rest of the closure is the
  # toolchain, stdenv and sources, several GB that cache.nixos.org already
  # serves.
  local nix_copy_build_outputs=""
  if [ "$copy_build_outputs" = "true" ]; then
    nix_copy_build_outputs="${nix_copy_cmd[@]} \$(nix-store -qR --include-outputs \"\$DRV\" | grep -v '[.]drv\$' | grep -- '-rust_')"
  fi

  # Don't actually run the command, just render it. We want the environment
  # variables from this machine, not the remote builder.
  echo "
  set -euxo pipefail
  DRV=\"\$(${nix_build_args[@]} | jq -r '.[0].drvPath')\"
  ${nix_copy_args[@]}
  $nix_copy_build_outputs
  "
}
export -f render_remote_cmd

function main() {
  local -r usage="USAGE: remote-execute-nix-build.sh <attr_path> <remote store> [copy build outputs]"
  local -r attr_path=${1?$usage}
  shift
  local -r remote_store=${1?$usage}
  shift
  local -r copy_build_outputs=${1:-false}

  printenv NIX_SUBSTITUTER_KEY >/tmp/secret-key

  scp \
    -o LogLevel=ERROR \
    -o "UserKnownHostsFile=$REMOTE_SERVER_USER_KNOWN_HOSTS_FILE" \
    /tmp/secret-key "github@$REMOTE_SERVER_ADDRESS:/tmp/nix-substituter-secret-key"

  # Execute the render_remote_cmd on the remote host, whilst also keeping a copy
  # of stdout on this machine.
  ssh "github@$REMOTE_SERVER_ADDRESS" \
    -o LogLevel=ERROR \
    -o "UserKnownHostsFile=$REMOTE_SERVER_USER_KNOWN_HOSTS_FILE" \
    -o SendEnv=AWS_ACCESS_KEY_ID \
    -o SendEnv=AWS_SECRET_ACCESS_KEY \
    -o SendEnv=AWS_SESSION_TOKEN \
    "bash -s" \
    < <(render_remote_cmd "$attr_path" "$remote_store" "$copy_build_outputs")
}
main "$@"
