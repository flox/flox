#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test 'flox delete'
#
# ---------------------------------------------------------------------------- #

load test_support.bash
# bats file_tags=delete

# ---------------------------------------------------------------------------- #

setup_file() {
  common_file_setup
  export FLOX_FEATURES_USE_CATALOG=true
  export  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.json"
}

teardown_file() {
  unset FLOX_FEATURES_USE_CATALOG
  unset _FLOX_USE_CATALOG_MOCK
}

# Helpers for project based tests.

project_setup() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/test"
  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" > /dev/null || return
}

project_teardown() {
  popd > /dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
}

# ---------------------------------------------------------------------------- #

setup() {
  common_test_setup
  setup_isolated_flox
  project_setup
}
teardown() {
  project_teardown
  common_test_teardown
}

dot_flox_exists() {
  # Since the return value is based on the exit code of `test`:
  # 0 = true
  # 1 = false
  [[ -d "$PROJECT_DIR/.flox" ]]
}

# ---------------------------------------------------------------------------- #

@test "deletes existing environment" {
  run "$FLOX_BIN" init
  assert_success
  run dot_flox_exists
  assert_success
  run "$FLOX_BIN" delete
  assert_success
  run dot_flox_exists
  assert_failure
}

# ---------------------------------------------------------------------------- #

@test "error message when called without .flox directory" {
  run dot_flox_exists
  assert_failure
  run "$FLOX_BIN" delete
  assert_failure
  assert_output --partial "Did not find an environment in the current directory."
}
