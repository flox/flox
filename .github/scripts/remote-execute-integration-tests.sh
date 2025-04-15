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

function upload_report_to_buildkite() {
  # Don't do anything if we're not in the merge queue or on main
  ! [[ 'merge_group' == "$GITHUB_EVENT_NAME" || 'main' == "$GITHUB_REF_NAME" ]] && return 0

  local -r git_commit_message="$(git log -1 --pretty=format:"%s")"
  local -r report_path_on_remote="$(awk '{ if ($1 == "TESTS_DIR:") { print $2 } }' output.txt)/report.xml"

  # Square bracket due to IPv6 being used to address the remote builderes via TailScale.
  scp \
    -6 \
    -o "UserKnownHostsFile=$REMOTE_SERVER_USER_KNOWN_HOSTS_FILE" \
    "github@[$REMOTE_SERVER_ADDRESS]:$report_path_on_remote" \
    ./report.xml

  # Remove all non-valid XML characters. We set -CSDA to make Perl use UTF8 for
  # its I/O and -p to make it loop over the whole file.
  perl -CSDA -pe 's/[^\P{C}\t\n\r]//g' ./report.xml > ./report-stripped.xml

  local -r report_path="$PWD/report-stripped.xml"

  curl \
    -X POST \
    --fail-with-body \
    -H "Authorization: Token token=\"$BUILDKITE_ANALYTICS_TOKEN\"" \
    -F "data=@$report_path" \
    -F "format=junit" \
    -F "run_env[CI]=github_actions" \
    -F "run_env[key]=$GITHUB_ACTION-$GITHUB_RUN_NUMBER-$GITHUB_RUN_ATTEMPT" \
    -F "run_env[number]=$GITHUB_RUN_NUMBER" \
    -F "run_env[branch]=$GITHUB_REF" \
    -F "run_env[commit_sha]=$GITHUB_SHA" \
    -F "run_env[message]=$git_commit_message" \
    -F "run_env[url]=https://github.com/$GITHUB_REPOSITORY/actions/runs/$GITHUB_RUN_ID" \
    -F "tags[architecture]=$MATRIX_SYSTEM" \
    -F "tags[nix_flakeref]=github:flox/flox/$GITHUB_SHA" \
    -F "tags[nix_attribute]=packages.$MATRIX_SYSTEM.flox-cli-tests" \
    https://analytics-api.buildkite.com/v1/uploads \
    || true
}
trap 'upload_report_to_buildkite' EXIT

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
}
main "$@"
