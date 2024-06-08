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
  export FLOX_FEATURES_USE_CATALOG=true
  export  _FLOX_USE_CATALOG_MOCK="$TEST_DATA_DIR/empty.json"
}

teardown_file() {
  unset FLOX_FEATURES_USE_CATALOG
  unset _FLOX_USE_CATALOG_MOCK
}

# ---------------------------------------------------------------------------- #

# Helpers for project based tests.

project_setup() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/project-${BATS_TEST_NUMBER?}"
  export PROJECT_NAME="${PROJECT_DIR##*/}"
  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" > /dev/null || return
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
