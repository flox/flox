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
  project_teardown
  common_test_teardown
}

# ---------------------------------------------------------------------------- #

# bats test_tags=auth,auth:login:notty
@test "auth login fails if not a tty" {
  run "$FLOX_BIN" auth login
  assert_failure
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
@test "auth login --token-file fails for a malformed token" {
  echo "not-a-jwt" > "$PROJECT_DIR/token"

  run "$FLOX_BIN" auth login --token-file "$PROJECT_DIR/token"
  assert_failure
  assert_output --partial "The provided token is not a valid FloxHub token."
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
