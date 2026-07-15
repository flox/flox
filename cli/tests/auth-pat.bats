#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test authentication with `flox_pat_` personal access tokens.
#
# A PAT is opaque: the CLI never parses it, and resolves the identity behind
# it lazily from `GET /api/v1/accounts/me`. These tests run a minimal local
# `/me` server (see auth/me_server.tcl) and point the CLI at it via
# FLOX_CATALOG_URL; push/pull traffic goes to the usual file-based floxhub.
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# bats file_tags=auth,auth:pat

# ---------------------------------------------------------------------------- #

setup_file() {
  common_file_setup
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.yaml"
}

teardown_file() {
  unset _FLOX_USE_CATALOG_MOCK
  common_file_teardown
}

# ---------------------------------------------------------------------------- #

project_setup() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/project-${BATS_TEST_NUMBER?}"
  export PROJECT_NAME="${PROJECT_DIR##*/}"
  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" > /dev/null || return
}

project_teardown() {
  popd > /dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
  unset PROJECT_NAME
}

# start_me_server <expected secret> <handle> <expires_at JSON>
#
# Starts the mock `/me` server and points the CLI's API base at it.
start_me_server() {
  ME_PORT_FILE="$BATS_TEST_TMPDIR/me_port"
  rm -f "$ME_PORT_FILE"
  MOCK_ME_SECRET="$1" \
  MOCK_ME_HANDLE="$2" \
  MOCK_ME_EXPIRES_AT="$3" \
  MOCK_ME_PORT_FILE="$ME_PORT_FILE" \
    expect -f "$TESTS_DIR/auth/me_server.tcl" \
    > "$BATS_TEST_TMPDIR/me_server.log" 2>&1 < /dev/null 3>&- &
  ME_SERVER_PID="$!"
  timeout 10 bash -c "while [ ! -s '$ME_PORT_FILE' ]; do sleep 0.1; done"
  export FLOX_CATALOG_URL="http://127.0.0.1:$(cat "$ME_PORT_FILE")"
}

stop_me_server() {
  if [ -n "${ME_SERVER_PID:-}" ]; then
    kill "$ME_SERVER_PID" 2> /dev/null || true
    unset ME_SERVER_PID
  fi
  unset FLOX_CATALOG_URL
}

setup() {
  common_test_setup
  setup_isolated_flox
  project_setup
  floxhub_setup "owner"
}

teardown() {
  stop_me_server
  project_teardown
  common_test_teardown
}

# ---------------------------------------------------------------------------- #

# bats test_tags=auth:pat:opaque
@test "pat: an opaque flox_pat_ token is not discarded as invalid" {
  export FLOX_FLOXHUB_TOKEN="flox_pat_test-secret"

  run "$FLOX_BIN" init
  assert_success
  refute_output --partial "token is invalid"
  refute_output --partial "token has expired"
}

# bats test_tags=auth:pat:owner
@test "pat: flox push resolves its default owner from /me" {
  export FLOX_FLOXHUB_TOKEN="flox_pat_test-secret"
  start_me_server "flox_pat_test-secret" "owner" "null"

  "$FLOX_BIN" init --name "test"
  run "$FLOX_BIN" push
  assert_success
  assert_output --partial "owner/test"
}

# bats test_tags=auth:pat:unauthorized
@test "pat: a rejected pat surfaces re-auth guidance" {
  export FLOX_FLOXHUB_TOKEN="flox_pat_revoked-secret"
  start_me_server "flox_pat_test-secret" "owner" "null"

  "$FLOX_BIN" init
  # Redirect stdin from /dev/null to ensure non-interactive mode
  run "$FLOX_BIN" push < /dev/null
  assert_failure
  assert_output --partial "Your FloxHub token has expired."
  assert_output --partial "To re-authenticate you can either"
}

# bats test_tags=auth:pat:expired
@test "pat: an expired pat surfaces re-auth guidance from the /me expiry" {
  export FLOX_FLOXHUB_TOKEN="flox_pat_test-secret"
  start_me_server "flox_pat_test-secret" "owner" '"2001-01-01T00:00:00Z"'

  "$FLOX_BIN" init
  # Redirect stdin from /dev/null to ensure non-interactive mode
  run "$FLOX_BIN" push < /dev/null
  assert_failure
  assert_output --partial "Your FloxHub token has expired."
}

# bats test_tags=auth:pat:degraded
@test "pat: an unreachable /me does not block calls with an explicit owner" {
  export FLOX_FLOXHUB_TOKEN="flox_pat_test-secret"
  # Nothing listens here: identity resolution fails, which must not be fatal.
  export FLOX_CATALOG_URL="http://127.0.0.1:1"

  "$FLOX_BIN" init --name "test"
  # The push itself goes to the file-based floxhub from floxhub_setup
  # (_FLOX_FLOXHUB_GIT_URL); /me is the only HTTP dependency in this test.
  run "$FLOX_BIN" push --owner "owner"
  assert_success
  assert_output --partial "owner/test"
}

# bats test_tags=auth:pat:login
@test "pat: auth login --token-file logs in with a pat" {
  unset FLOX_FLOXHUB_TOKEN
  start_me_server "flox_pat_test-secret" "owner" "null"
  echo "flox_pat_test-secret" > "$BATS_TEST_TMPDIR/token"

  run "$FLOX_BIN" auth login --token-file "$BATS_TEST_TMPDIR/token"
  assert_success
  assert_output --partial "Logged in as owner"
}

# bats test_tags=auth:pat:login:revoked
@test "pat: auth login --token-file rejects a revoked pat" {
  unset FLOX_FLOXHUB_TOKEN
  start_me_server "flox_pat_test-secret" "owner" "null"
  echo "flox_pat_revoked" > "$BATS_TEST_TMPDIR/token"

  run "$FLOX_BIN" auth login --token-file "$BATS_TEST_TMPDIR/token"
  assert_failure
  assert_output --partial "The provided token is expired."
}
