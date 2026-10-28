#!/usr/bin/env bash

# Build a package on this runner and publish its closure, the local
# counterpart of remote-execute-nix-build.sh.

set -euxo pipefail

function main() {
  local -r usage="USAGE: local-nix-build.sh <attr_path> <remote store> [<build cache>]"
  local -r attr_path=${1?$usage}
  shift
  local -r remote_store=${1?$usage}
  shift
  local -r build_cache=${1:-}

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

  # The copy above only publishes the product closures. Everything built
  # along the way (toolchains, test dependencies) otherwise lives only on
  # the sticky disk, so also push the build-time closure to the CI cache,
  # minus what cache.nixos.org already serves. A skipped path is still
  # uploaded when a pushed path references it, since a binary cache only
  # accepts paths whose references it holds. Best effort: a failed push
  # must not fail the build, and if the cache.nixos.org lookup fails
  # nothing is skipped.
  if [ -n "$build_cache" ]; then
    (
      paths=$(mktemp)
      cached=$(mktemp)
      nix-store --query --requisites --include-outputs "$drv" \
        | grep -v '\.drv$' >"$paths"
      awk -F/ '{ printf "url = \"https://cache.nixos.org/%s.narinfo\"\noutput = \"/dev/null\"\n", substr($4, 1, 32) }' "$paths" \
        | curl --silent --head --parallel --parallel-max 32 --config - \
            --write-out '%{http_code} %{url_effective}\n' \
        | awk '$1 == 200 { n = split($2, p, "/"); sub(/\.narinfo$/, "", p[n]); print p[n] }' >"$cached" \
        || true
      echo "Pushing $(grep -cvFf "$cached" "$paths") of $(grep -c '' "$paths") build-time paths (rest are on cache.nixos.org)"

      { set +x; } 2>/dev/null
      unset AWS_SESSION_TOKEN
      export AWS_ACCESS_KEY_ID="$AWS_R2_ACCESS_KEY_ID"
      export AWS_SECRET_ACCESS_KEY="$AWS_R2_SECRET_ACCESS_KEY"
      set -x
      { grep -vFf "$cached" "$paths" || true; } \
        | nix copy --extra-experimental-features nix-command --stdin \
            --to "$build_cache&secret-key=/tmp/secret-key"
    ) || echo "::warning::Failed to push the build-time closure to the CI cache"
  fi
}
main "$@"
