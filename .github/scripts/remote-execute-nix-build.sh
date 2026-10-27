#!/usr/bin/env bash

set -euxo pipefail

function render_remote_cmd() {
  local -r flake_ref="github:flox/flox/$GITHUB_SHA#$1"
  shift
  local -r remote_store=$1
  shift
  local -r build_cache=${1:-}

  local -r nix_build_args=(
    nix build
    --extra-experimental-features '"nix-command flakes"'
    --no-link
    --print-build-logs
    --json
    --accept-flake-config
    "$flake_ref")

  local -r nix_copy_args=(
    nix copy
    --extra-experimental-features nix-command
    --substitute-on-destination
    --to $remote_store?secret-key=/tmp/nix-substituter-secret-key
    \$DRV
    \$DRV^*
  )

  # Don't actually run the command, just render it. We want the environment
  # variables from this machine, not the remote builder.
  echo "
  set -euxo pipefail
  DRV=\"\$(${nix_build_args[@]} | jq -r '.[0].drvPath')\"
  ${nix_copy_args[@]}
  "

  # The copy above only publishes the product closures. Everything built
  # along the way (toolchains, test dependencies) otherwise lives only in
  # the builder's store until it is garbage collected, so also push the
  # build-time closure to the CI cache. Best effort: a failed push must not
  # fail the build.
  if [ -n "$build_cache" ]; then
    local -r build_cache_url=$(printf '%q' "$build_cache&secret-key=/tmp/nix-substituter-secret-key")
    # shellcheck disable=SC2016 # expanded on the remote builder
    echo '
  (
    { set +x; } 2>/dev/null
    unset AWS_SESSION_TOKEN
    export AWS_ACCESS_KEY_ID="$AWS_R2_ACCESS_KEY_ID"
    export AWS_SECRET_ACCESS_KEY="$AWS_R2_SECRET_ACCESS_KEY"
    set -x
    nix-store --query --requisites --include-outputs "$DRV" \
      | grep -v "\.drv$" \
      | nix copy --extra-experimental-features nix-command --stdin \
          --to '"$build_cache_url"'
  ) || echo "::warning::Failed to push the build-time closure to the CI cache"
  '
  fi
}
export -f render_remote_cmd

function main() {
  local -r usage="USAGE: remote-execute-nix-build.sh <attr_path> <remote store> [<build cache>]"
  local -r attr_path=${1?$usage}
  shift
  local -r remote_store=${1?$usage}
  shift
  local -r build_cache=${1:-}

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
    -o SendEnv=AWS_R2_ACCESS_KEY_ID \
    -o SendEnv=AWS_R2_SECRET_ACCESS_KEY \
    "bash -s" \
    < <(render_remote_cmd "$attr_path" "$remote_store" "$build_cache")
}
main "$@"
