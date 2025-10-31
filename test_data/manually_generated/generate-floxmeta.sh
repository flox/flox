#!/usr/bin/env bash

set -euxo pipefail

FLOX_TAG="${1?}"
shift
TESTS_DIR="${1?}"
shift

BATS_TEST_TMPDIR="$(mktemp -d)"
OWNER="owner"
NAME="name"
_FLOX_ACTIVE_ENVIRONMENTS=
FLOX="$(nix build github:flox/flox/$FLOX_TAG --json --no-link | jq '.[0].outputs.out' -r)/bin/flox"



pushd "$BATS_TEST_TMPDIR" || exit
bats_load_library() {
  :
}
bats_require_minimum_version() {
  :
}
source "$TESTS_DIR/setup_suite.bash"
home_setup test
load() {
  :
}
source "$TESTS_DIR/test_support.bash"
floxhub_setup "$OWNER"
pushd "$BATS_TEST_TMPDIR" || exit
"$FLOX" init --name "$NAME"
"$FLOX" push --owner "$OWNER"
"$FLOX" delete -f

# Massage the current yaml mock format into the older JSON format
yq '.then.body' "$GENERATED_DATA/resolve/hello.yaml" -r | jq '[[.items[] | { msgs: .messages, name, page: (.page | .msgs = .messages | del(.messages)) } ]]'> hello.json
_FLOX_USE_CATALOG_MOCK="hello.json" \
  "$FLOX" install hello --remote "$OWNER/$NAME"

echo "$BATS_TEST_TMPDIR"

mkdir -p "$MANUALLY_GENERATED/with-$FLOX_TAG"
tar -C "$BATS_TEST_TMPDIR/floxhub/owner/floxmeta" -czf "$MANUALLY_GENERATED/with-$FLOX_TAG/floxmeta.tar.gz" .
