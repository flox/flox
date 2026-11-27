#!/usr/bin/env bash

# Run the bats integration tests on this runner, the local counterpart of
# remote-execute-integration-tests.sh. Leaves the JUnit report at
# ./report.xml, like the remote script.

set -euxo pipefail

function main() {
  local -r flake_ref=".#packages.$MATRIX_SYSTEM.flox-cli-tests"

  # Keep a copy of stdout: the test runner prints the temporary directory it
  # used as TESTS_DIR, and bats writes the JUnit report.xml there.
  nix run --accept-flake-config "$flake_ref" \
    -- --ci-runner "flox-$MATRIX_SYSTEM" \
    -- --filter-tags "$MATRIX_TEST_TAGS" --report-formatter junit \
    | tee output.txt

  local -r report="$(awk '{ if ($1 == "TESTS_DIR:") { print $2 } }' output.txt)/report.xml"
  cp "$report" ./report.xml
}
main "$@"
