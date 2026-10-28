#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test that we can have an authentication flow
#
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# bats file_tags=auth

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

# Helpers for project based tests.

project_setup() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/project-${BATS_TEST_NUMBER?}"
  export PROJECT_NAME="${PROJECT_DIR##*/}"
  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" > /dev/null || return
  # Tests in this file assert on login state persisted in FLOX_CONFIG_DIR.
  # Bats runs tests within a file in parallel on CI, so each test needs its
  # own config dir — otherwise a neighboring test's 'auth logout' (below)
  # removes the token another test just stored.
  setup_isolated_flox
  unset FLOX_FLOXHUB_TOKEN
  "$FLOX_BIN" auth logout
}

project_teardown() {
  popd > /dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
  unset PROJECT_NAME
}

# ---------------------------------------------------------------------------- #

setup() {
  common_test_setup
  project_setup
}

teardown() {
  if [ -n "${DEVICE_GRANT_PID:-}" ]; then
    kill "$DEVICE_GRANT_PID"
  fi
  project_teardown
  common_test_teardown
}

# ---------------------------------------------------------------------------- #

# Serve the OAuth device grant from tests/auth/device_grant_server.py and
# point 'flox auth login' at it, so login tests never reach a real issuer.
start_device_grant_server() {
  local port_file="$BATS_TEST_TMPDIR/device-grant-port"
  python3 "$TESTS_DIR/auth/device_grant_server.py" "$port_file" "$DUMMY_TOKEN" 3>&- &
  DEVICE_GRANT_PID=$!
  timeout 10 bash -c "until [ -s '$port_file' ]; do sleep 0.1; done"
  DEVICE_GRANT_URL="http://127.0.0.1:$(cat "$port_file")"
  export FLOX_FLOXHUB_URL="$DEVICE_GRANT_URL"
  export _FLOX_OAUTH_DEVICE_AUTH_URL="$DEVICE_GRANT_URL/device"
  export _FLOX_OAUTH_TOKEN_URL="$DEVICE_GRANT_URL/token"
}

# wait_for_output FILE TEXT
# Wait up to 10 seconds for TEXT to appear in FILE.
wait_for_output() {
  timeout 10 bash -c "until grep -qF '$2' '$1' 2> /dev/null; do sleep 0.1; done"
}

# bats test_tags=auth,auth:login:notty
@test "auth login without a tty prints the URL and waits for approval" {
  start_device_grant_server

  "$FLOX_BIN" auth login < /dev/null > "$PROJECT_DIR/login.out" 2>&1 3>&- &
  login_pid=$!
  wait_for_output "$PROJECT_DIR/login.out" "Waiting for approval"
  curl -fsS "$DEVICE_GRANT_URL/approve"
  wait "$login_pid"

  run cat "$PROJECT_DIR/login.out"
  assert_line "Your one-time activation code is: CODE-1"
  assert_line "$DEVICE_GRANT_URL/activate?user_code=CODE-1"
  assert_line "Waiting for approval. The code expires in 15 minutes."
  assert_output --partial "Logged in as test"

  run "$FLOX_BIN" auth status
  assert_success
  assert_output --partial "You are logged in as test"
}

# ---------------------------------------------------------------------------- #

# A valid dummy JWT with payload:
#   { "https://flox.dev/handle": "test", "exp": 9999999999 }
# (same token as floxhub_setup in test_support.bash)
DUMMY_TOKEN="eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.eyJodHRwczovL2Zsb3guZGV2L2hhbmRsZSI6InRlc3QiLCJleHAiOjk5OTk5OTk5OTl9.6-nbzFzQEjEX7dfWZFLE-I_qW2N_-9W2HFzzfsquI74"

# An expired dummy JWT with payload:
#   { "https://flox.dev/handle": "test", "exp": 1704063600 }
EXPIRED_TOKEN="eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.eyJodHRwczovL2Zsb3guZGV2L2hhbmRsZSI6InRlc3QiLCJleHAiOjE3MDQwNjM2MDB9.-5VCofPtmYQuvh21EV1nEJhTFV_URkRP0WFu4QDPFxY"

# bats test_tags=auth,auth:login:token-file
@test "auth login --token-file logs in with a token from a file" {
  echo "$DUMMY_TOKEN" > "$PROJECT_DIR/token"

  run "$FLOX_BIN" auth login --token-file "$PROJECT_DIR/token"
  assert_success
  assert_output --partial "Logged in as test"

  run "$FLOX_BIN" auth status
  assert_success
  assert_output --partial "You are logged in as test"

  run "$FLOX_BIN" auth token
  assert_success
  assert_output "$DUMMY_TOKEN"
}

# bats test_tags=auth,auth:login:token-file
@test "auth login --token-file - reads the token from stdin" {
  run sh -c "echo '$DUMMY_TOKEN' | '$FLOX_BIN' auth login --token-file -"
  assert_success
  assert_output --partial "Logged in as test"

  run "$FLOX_BIN" auth token
  assert_success
  assert_output "$DUMMY_TOKEN"
}

# bats test_tags=auth,auth:login:token-file
@test "auth login --token-file fails for a missing file" {
  run "$FLOX_BIN" auth login --token-file "$PROJECT_DIR/does-not-exist"
  assert_failure
  assert_output --partial "Could not read token file"
}

# bats test_tags=auth,auth:login:token-file
@test "auth login --token-file fails for a token the server rejects" {
  # A non-JWT token cannot be rejected locally — it may be an issuer's
  # opaque access token — so it is verified via /me, which answers 401 here.
  export _FLOX_USE_CATALOG_MOCK="$MANUALLY_GENERATED/auth/me_revoked.yaml"
  echo "not-a-jwt" > "$PROJECT_DIR/token"

  run "$FLOX_BIN" auth login --token-file "$PROJECT_DIR/token"
  assert_failure
  assert_output --partial "FloxHub rejected the provided token: it is invalid, expired, or revoked."
}

# bats test_tags=auth
@test "an opaque stored token is not discarded as invalid" {
  # No token can be rejected locally — an issuer may mint opaque access
  # tokens — so a non-JWT stored token is kept, counts as logged in like a
  # flox_pat_ token, and the server stays the authority for its validity.
  echo 'floxhub_token = "not-a-jwt"' >> "$FLOX_CONFIG_DIR/flox.toml"

  run "$FLOX_BIN" config
  assert_success
  refute_output --partial "token is invalid"
  refute_output --partial "not logged in to FloxHub"
  run grep floxhub_token "$FLOX_CONFIG_DIR/flox.toml"
  assert_success
}

# bats test_tags=auth,auth:login:token-file
@test "auth login --token-file fails for an expired token" {
  echo "$EXPIRED_TOKEN" > "$PROJECT_DIR/token"

  run "$FLOX_BIN" auth login --token-file "$PROJECT_DIR/token"
  assert_failure
  assert_output --partial "The provided token is expired."

  run "$FLOX_BIN" auth status
  assert_failure
}

# bats test_tags=auth,auth:cache
@test "plaintext login caches identity without claiming a keyring credential" {
  echo "$DUMMY_TOKEN" > "$PROJECT_DIR/token"
  run "$FLOX_BIN" auth login --token-file "$PROJECT_DIR/token" --insecure-storage
  assert_success

  run jq -c '[.storage, .handle, .logged_in]' "$FLOX_CACHE_DIR"/auth-state-*.json
  assert_success
  assert_output '["plaintext","test",true]'

  # Removing the plaintext token must not leave its identity posing as
  # a credential in the (suite-wide disabled) keyring on the next invocation.
  run "$FLOX_BIN" config --delete floxhub_token
  assert_success
  # The delete command resolved auth before removing the token. Start a new
  # invocation to check that auth resolution replaces the stale cache record.
  run "$FLOX_BIN" config
  assert_success

  run jq -c '[.storage, .handle, .logged_in]' "$FLOX_CACHE_DIR"/auth-state-*.json
  assert_success
  assert_output '["keyring",null,false]'
}

# bats test_tags=auth,auth:cache
@test "logout caches the empty keyring in the cache dir" {
  echo "$DUMMY_TOKEN" > "$PROJECT_DIR/token"
  run "$FLOX_BIN" auth login --token-file "$PROJECT_DIR/token" --insecure-storage
  assert_success

  run jq -c '[.storage, .handle, .logged_in]' "$FLOX_CACHE_DIR"/auth-state-*.json
  assert_success
  assert_output '["plaintext","test",true]'

  run "$FLOX_BIN" auth logout
  assert_success
  run jq -c '[.storage, .handle, .logged_in]' "$FLOX_CACHE_DIR"/auth-state-*.json
  assert_success
  assert_output '["keyring",null,false]'
}
