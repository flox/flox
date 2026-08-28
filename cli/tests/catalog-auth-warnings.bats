#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Tests for DEV-199: warn unauthenticated users at the point of contacting the
# catalog /resolve endpoint — the call that will require authentication once
# catalog auth gating is enforced server-side.
#
# Commands that never resolve (fully locked environments, empty manifests)
# must stay quiet; any command that triggers a resolve warns, rate-limited to
# once per 8 hours via a timestamp file in the cache directory.
#
# bats file_tags=catalog-auth-warnings
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# ---------------------------------------------------------------------------- #

RESOLVE_AUTH_WARNING="Resolving packages will require authentication to FloxHub in an upcoming release."
RESOLVE_AUTH_DOCS_URL="https://go.flox.dev/auth"
STAMP_FILE_NAME="resolve-auth-warning-timestamp.json"

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
# Warn on resolve
# ---------------------------------------------------------------------------- #

@test "resolving while logged out prints the auth warning" {
  skip_x86_64_darwin_replay
  # The suite runs "logged in" by default; this test needs the logged-out state.
  unset FLOX_FLOXHUB_TOKEN
  "$FLOX_BIN" init
  run "$FLOX_BIN" install hello
  assert_success
  assert_output --partial "$RESOLVE_AUTH_WARNING"
}

# DEV-236: the warning must carry the auth explainer link, not the install docs.
@test "the auth warning links to the auth explainer page" {
  skip_x86_64_darwin_replay
  unset FLOX_FLOXHUB_TOKEN
  "$FLOX_BIN" init
  run "$FLOX_BIN" install hello
  assert_success
  assert_output --partial "$RESOLVE_AUTH_DOCS_URL"
  refute_output --partial "flox.dev/docs/install-flox"
}

@test "resolving while logged in does NOT print the auth warning" {
  skip_x86_64_darwin_replay
  "$FLOX_BIN" init
  run "$FLOX_BIN" install hello
  assert_success
  refute_output --partial "$RESOLVE_AUTH_WARNING"
}

@test "'-q' suppresses the auth warning" {
  skip_x86_64_darwin_replay
  unset FLOX_FLOXHUB_TOKEN
  "$FLOX_BIN" init
  run "$FLOX_BIN" -q install hello
  assert_success
  refute_output --partial "$RESOLVE_AUTH_WARNING"
}

@test "'-q' does not consume the rate-limit window" {
  skip_x86_64_darwin_replay
  unset FLOX_FLOXHUB_TOKEN
  "$FLOX_BIN" init
  run "$FLOX_BIN" -q install hello
  refute_output --partial "$RESOLVE_AUTH_WARNING"
  # The quiet invocation must not have written the stamp: the next
  # interactive resolve still warns.
  rm .flox/env/manifest.lock
  run "$FLOX_BIN" list
  assert_success
  assert_output --partial "$RESOLVE_AUTH_WARNING"
}

@test "'auth_notifications = false' suppresses the auth warning" {
  skip_x86_64_darwin_replay
  unset FLOX_FLOXHUB_TOKEN
  mkdir -p "$FLOX_CONFIG_DIR"
  echo 'auth_notifications = false' >> "$FLOX_CONFIG_DIR/flox.toml"
  "$FLOX_BIN" init
  run "$FLOX_BIN" install hello
  assert_success
  refute_output --partial "$RESOLVE_AUTH_WARNING"
}

@test "'auth_notifications = false' does not consume the rate-limit window" {
  skip_x86_64_darwin_replay
  unset FLOX_FLOXHUB_TOKEN
  mkdir -p "$FLOX_CONFIG_DIR"
  echo 'auth_notifications = false' >> "$FLOX_CONFIG_DIR/flox.toml"
  "$FLOX_BIN" init
  run "$FLOX_BIN" install hello
  refute_output --partial "$RESOLVE_AUTH_WARNING"
  # Quieting must not have written the stamp: a user who turns the key back
  # on still gets the next warning rather than an already-consumed window.
  rm .flox/env/manifest.lock
  rm "$FLOX_CONFIG_DIR/flox.toml"
  run "$FLOX_BIN" list
  assert_success
  assert_output --partial "$RESOLVE_AUTH_WARNING"
}

@test "resolving with an expired token prints the auth warning" {
  skip_x86_64_darwin_replay
  # Same shape as the suite token but with exp in the past (2001-09-09).
  export FLOX_FLOXHUB_TOKEN="eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.eyJodHRwczovL2Zsb3guZGV2L2hhbmRsZSI6InRlc3QiLCJleHAiOjEwMDAwMDAwMDB9.6-nbzFzQEjEX7dfWZFLE-I_qW2N_-9W2HFzzfsquI74"
  "$FLOX_BIN" init
  run "$FLOX_BIN" install hello
  assert_success
  assert_output --partial "$RESOLVE_AUTH_WARNING"
}

# ---------------------------------------------------------------------------- #
# No resolve, no warning
# ---------------------------------------------------------------------------- #

@test "'flox init' with nothing to resolve does NOT print the auth warning" {
  unset FLOX_FLOXHUB_TOKEN
  run "$FLOX_BIN" init
  assert_success
  refute_output --partial "$RESOLVE_AUTH_WARNING"
}

@test "'flox activate' with existing lockfile does NOT print the auth warning" {
  skip_x86_64_darwin_replay
  # Lock the environment while logged in, then activate logged out: the
  # lockfile means no resolve happens, so no warning — the case that must
  # stay quiet for `flox activate` in shell rc files.
  "$FLOX_BIN" init
  "$FLOX_BIN" install hello
  unset FLOX_FLOXHUB_TOKEN
  run "$FLOX_BIN" activate -- true
  assert_success
  refute_output --partial "$RESOLVE_AUTH_WARNING"
}

# ---------------------------------------------------------------------------- #
# Rate limiting
# ---------------------------------------------------------------------------- #

@test "a second resolve within the rate-limit window does NOT warn again" {
  skip_x86_64_darwin_replay
  unset FLOX_FLOXHUB_TOKEN
  "$FLOX_BIN" init
  run "$FLOX_BIN" install hello
  assert_output --partial "$RESOLVE_AUTH_WARNING"
  # Force a fresh resolve by removing the lockfile.
  rm .flox/env/manifest.lock
  run "$FLOX_BIN" list
  assert_success
  refute_output --partial "$RESOLVE_AUTH_WARNING"
}

@test "a resolve after the rate-limit window expires warns again" {
  skip_x86_64_darwin_replay
  unset FLOX_FLOXHUB_TOKEN
  "$FLOX_BIN" init
  "$FLOX_BIN" install hello
  # Backdate the stamp beyond the 8 hour window.
  echo '{"last_warning":"2020-01-01T00:00:00Z"}' > "$FLOX_CACHE_DIR/$STAMP_FILE_NAME"
  rm .flox/env/manifest.lock
  run "$FLOX_BIN" list
  assert_success
  assert_output --partial "$RESOLVE_AUTH_WARNING"
}
