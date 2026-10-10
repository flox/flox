#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# The catalog decides whether resolving packages requires authentication: the
# CLI always sends the request, and turns a 401 into a login prompt. Logged-out
# resolution keeps working while the catalog allows it, and commands that
# resolve nothing (fully locked environments) never ask.
#
# bats file_tags=catalog-auth-required
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# ---------------------------------------------------------------------------- #

# The first line is the `detail` of the catalog's 401 in the mock.
AUTH_REQUIRED="Authentication is required to resolve packages."
UNAUTHORIZED_MOCK="$MANUALLY_GENERATED/resolve/unauthorized.yaml"
# A 401 with the catalog's reason for rejecting a token.
REJECTED_MOCK="$MANUALLY_GENERATED/resolve/rejected.yaml"
# A 401 from the store info endpoint, triggered during custom-package download.
STORE_UNAUTHORIZED_MOCK="$MANUALLY_GENERATED/store/unauthorized.yaml"
# Fixture: a locked environment containing a custom-catalog package whose store
# paths are absent from the local Nix store, so activation must consult the
# catalog for download locations.
CUSTOM_CATALOG_HELLO_DIR="$MANUALLY_GENERATED/custom_catalog_hello"

assert_auth_required_error() {
  assert_output --partial - << 'EOF'
✘ ERROR: Authentication is required to resolve packages.
For CI and automation, see https://go.flox.dev/auth
Log in with 'flox auth login'.
EOF
}

assert_login_expired_error() {
  assert_output --partial - << 'EOF'
✘ ERROR: Your FloxHub login has expired.
For CI and automation, see https://go.flox.dev/auth
Log in again with 'flox auth login'.
EOF
}

assert_login_rejected_error() {
  assert_output --partial - << 'EOF'
✘ ERROR: Authentication rejected: Unable to verify token
For CI and automation, see https://go.flox.dev/auth
Log in again with 'flox auth login'.
EOF
}

project_setup() {
  export PROJECT_NAME="test"
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/$PROJECT_NAME"
  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" > /dev/null || return
}

project_teardown() {
  popd > /dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
}

setup() {
  common_test_setup
  setup_isolated_flox
  project_setup
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.yaml"
}

teardown() {
  project_teardown
  common_test_teardown
}

# ---------------------------------------------------------------------------- #
# The catalog requires authentication
# ---------------------------------------------------------------------------- #

@test "resolving while logged out shows the catalog's login message" {
  # The suite runs "logged in" by default; this test needs the logged-out state.
  unset FLOX_FLOXHUB_TOKEN
  "$FLOX_BIN" init
  export _FLOX_USE_CATALOG_MOCK="$UNAUTHORIZED_MOCK"
  run "$FLOX_BIN" install hello
  assert_failure
  assert_auth_required_error
}

# The catalog treats an expired token as no token at all; the CLI knows the
# expiry, so it says why login is needed.
@test "resolving with an expired token says the login expired" {
  # Same shape as the suite token but with exp in the past (2001-09-09).
  export FLOX_FLOXHUB_TOKEN="eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.eyJodHRwczovL2Zsb3guZGV2L2hhbmRsZSI6InRlc3QiLCJleHAiOjEwMDAwMDAwMDB9.6-nbzFzQEjEX7dfWZFLE-I_qW2N_-9W2HFzzfsquI74"
  "$FLOX_BIN" init
  export _FLOX_USE_CATALOG_MOCK="$UNAUTHORIZED_MOCK"
  run "$FLOX_BIN" install hello
  assert_failure
  assert_login_expired_error
}

@test "resolving with a rejected login says the login was rejected" {
  "$FLOX_BIN" init
  export _FLOX_USE_CATALOG_MOCK="$REJECTED_MOCK"
  run "$FLOX_BIN" install hello
  assert_failure
  assert_login_rejected_error
  refute_output --partial "401"
}

@test "'flox activate' that needs to lock shows the catalog's login message" {
  unset FLOX_FLOXHUB_TOKEN
  "$FLOX_BIN" init
  with_latest_schema '[install]
hello.pkg-path = "hello"' > .flox/env/manifest.toml
  export _FLOX_USE_CATALOG_MOCK="$UNAUTHORIZED_MOCK"
  run "$FLOX_BIN" activate -- true
  assert_failure
  assert_auth_required_error
}

@test "'flox run' shows the catalog's login message" {
  unset FLOX_FLOXHUB_TOKEN
  export _FLOX_USE_CATALOG_MOCK="$UNAUTHORIZED_MOCK"
  run "$FLOX_BIN" run --package hello hello
  assert_failure
  assert_auth_required_error
  refute_output --partial "Check your network connection"
}

# When a custom-catalog package is not in the local Nix store, activate must
# call the catalog for download locations. A 401 from that endpoint should
# produce a clean 'flox auth login' prompt rather than a raw API error.
@test "'flox activate' with custom catalog package shows login prompt when logged out" {
  unset FLOX_FLOXHUB_TOKEN
  "$FLOX_BIN" init
  cp "$CUSTOM_CATALOG_HELLO_DIR/manifest.toml" .flox/env/manifest.toml
  cp "$CUSTOM_CATALOG_HELLO_DIR/manifest.lock" .flox/env/manifest.lock
  export _FLOX_USE_CATALOG_MOCK="$STORE_UNAUTHORIZED_MOCK"
  run "$FLOX_BIN" activate -- true
  assert_failure
  assert_output --partial "flox auth login"
  refute_output --partial "Unexpected error calling the catalog client"
}

# ---------------------------------------------------------------------------- #
# The catalog allows the request
# ---------------------------------------------------------------------------- #

# The CLI must not refuse by itself, so the catalog can stop requiring
# authentication without a CLI release.
@test "resolving while logged out succeeds when the catalog allows it" {
  skip_x86_64_darwin_replay
  unset FLOX_FLOXHUB_TOKEN
  "$FLOX_BIN" init
  run "$FLOX_BIN" install hello
  assert_success
  refute_output --partial "$AUTH_REQUIRED"
}

@test "'flox activate' with existing lockfile succeeds while logged out" {
  skip_x86_64_darwin_replay
  # Lock the environment while logged in, then activate logged out: the
  # lockfile means the catalog isn't contacted at all — the case that must
  # keep working for `flox activate` in shell rc files. The empty mock fails
  # the test on any catalog request.
  "$FLOX_BIN" init
  "$FLOX_BIN" install hello
  unset FLOX_FLOXHUB_TOKEN
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.yaml"
  run "$FLOX_BIN" activate -- true
  assert_success
  refute_output --partial "$AUTH_REQUIRED"
}
