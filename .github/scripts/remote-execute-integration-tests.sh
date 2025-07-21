#!/usr/bin/env bash

set -euxo pipefail

function render_remote_cmd() {
  local -r flake_ref="github:flox/flox/$GITHUB_SHA#packages.$MATRIX_SYSTEM.flox-cli-tests"
  local -r nix_args=(--accept-flake-config --extra-experimental-features '"nix-command flakes"' "$flake_ref")
  local -r ci_args=(--ci-runner "flox-$MATRIX_SYSTEM")
  local -r bats_args=(--filter-tags "$MATRIX_TEST_TAGS" --report-formatter junit)

  # Don't actually run the command, just render it. We want the environment
  # variables from this machine, not the remote builder.
  echo nix run "${nix_args[@]}" -- "${ci_args[@]}" -- "${bats_args[@]}"
}
export -f render_remote_cmd

function retrieve_report_from_remote() {
  # Square bracket due to IPv6 being used to address the remote builderes via TailScale.
  local -r report_path_on_remote="$(awk '{ if ($1 == "TESTS_DIR:") { print $2 } }' output.txt)/report.xml"
  scp \
    -6 \
    -o "UserKnownHostsFile=$REMOTE_SERVER_USER_KNOWN_HOSTS_FILE" \
    "github@[$REMOTE_SERVER_ADDRESS]:$report_path_on_remote" \
    ./report.xml
}

function main() {
  git clean -xfd

  # Execute the render_remote_cmd on the remote host, whilst also keeping a copy
  # of stdout on this machine. We'll use that output.txt later to extract which
  # temporary directory was used as WORKDIR when running the tests, since bats
  # will output the JUnit report.xml there.
  ssh "github@$REMOTE_SERVER_ADDRESS" \
    -o LogLevel=ERROR \
    -o "UserKnownHostsFile=$REMOTE_SERVER_USER_KNOWN_HOSTS_FILE" \
    "$(render_remote_cmd)" \
    | tee output.txt

  # Retrive report.xml
  retrieve_report_from_remote
}
main "$@"
