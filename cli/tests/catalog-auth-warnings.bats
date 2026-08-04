#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Tests for DEV-199: per-command deprecation warnings before catalog auth
# gating is enforced.
#
# bats file_tags=catalog-auth-warnings
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# ---------------------------------------------------------------------------- #

AUTH_WARNING="This command will require authentication in an upcoming release."
LOCK_WARNING="Locking environments will require authentication in an upcoming release."

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
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.yaml"
}

teardown() {
  project_teardown
  common_test_teardown
}

# ---------------------------------------------------------------------------- #
# Unconditional warnings
# ---------------------------------------------------------------------------- #

@test "'flox init' prints auth deprecation warning when logged out" {
  # The suite runs "logged in" by default; this test needs the logged-out state.
  unset FLOX_FLOXHUB_TOKEN
  run "$FLOX_BIN" init
  assert_success
  assert_output --partial "$AUTH_WARNING"
}

@test "'flox init' suppresses auth warning when logged in" {
  floxhub_setup "owner"
  run "$FLOX_BIN" init
  assert_success
  refute_output --partial "$AUTH_WARNING"
}

# ---------------------------------------------------------------------------- #
# Conditional warning: activate
# ---------------------------------------------------------------------------- #

@test "'flox activate' with existing lockfile does NOT print lock warning" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.yaml"
  "$FLOX_BIN" init
  "$FLOX_BIN" install hello
  # First activate creates the lockfile
  "$FLOX_BIN" activate -c true
  # Second activate: lockfile already exists — no warning expected
  run "$FLOX_BIN" activate -c true
  assert_success
  refute_output --partial "$LOCK_WARNING"
}

@test "'flox activate' without lockfile prints lock warning" {
  # The suite runs "logged in" by default; this test needs the logged-out state.
  unset FLOX_FLOXHUB_TOKEN
  "$FLOX_BIN" init
  # flox init always creates a lockfile; remove it to simulate a pre-lockfile env
  rm .flox/env/manifest.lock
  run "$FLOX_BIN" activate -c true
  assert_success
  assert_output --partial "$LOCK_WARNING"
}

# ---------------------------------------------------------------------------- #
# Conditional warning: list
# ---------------------------------------------------------------------------- #

@test "'flox list' with existing lockfile does NOT print lock warning" {
  unset FLOX_FLOXHUB_TOKEN
  "$FLOX_BIN" init
  # flox init creates a lockfile; list should NOT warn because lockfile exists
  run "$FLOX_BIN" list
  assert_success
  # May print a "no packages" warning, but must not print the lock warning
  refute_output --partial "$LOCK_WARNING"
}

@test "'flox list' without lockfile prints lock warning" {
  # The suite runs "logged in" by default; this test needs the logged-out state.
  unset FLOX_FLOXHUB_TOKEN
  "$FLOX_BIN" init
  # flox init always creates a lockfile; remove it to simulate a pre-lockfile env
  rm .flox/env/manifest.lock
  run "$FLOX_BIN" list
  assert_success
  assert_output --partial "$LOCK_WARNING"
}

# ---------------------------------------------------------------------------- #
# Conditional warning: build
# ---------------------------------------------------------------------------- #

@test "'flox build' with existing lockfile does NOT print lock warning" {
  "$FLOX_BIN" init
  # flox init creates a lockfile; build should NOT warn because lockfile exists
  run "$FLOX_BIN" build
  # Command fails because there are no build targets, but the lock warning
  # must not appear — the lockfile gate prevents it regardless of auth state.
  refute_output --partial "$LOCK_WARNING"
}

@test "'flox build' without lockfile prints lock warning" {
  # The suite runs "logged in" by default; this test needs the logged-out state.
  unset FLOX_FLOXHUB_TOKEN
  "$FLOX_BIN" init
  # flox init always creates a lockfile; remove it to simulate a pre-lockfile env
  rm .flox/env/manifest.lock
  run "$FLOX_BIN" build
  # Command fails because there are no build targets, but the lock warning
  # must appear before the build attempt when unauthenticated and no lockfile.
  assert_output --partial "$LOCK_WARNING"
}
