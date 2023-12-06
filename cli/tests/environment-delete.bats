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

# Helpers for project based tests.

project_setup() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/test"
  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" >/dev/null || return
}

project_teardown() {
  popd >/dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
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

dot_flox_exists() {
  # Since the return value is based on the exit code of `test`:
  # 0 = true
  # 1 = false
  [[ -d "$PROJECT_DIR/.flox" ]];
}

# ---------------------------------------------------------------------------- #

@test "deletes existing environment" {
  run "$FLOX_CLI" init;
  assert_success;
  run dot_flox_exists;
  assert_success;
  run "$FLOX_CLI" delete;
  assert_success;
  run dot_flox_exists;
  assert_failure;
}


# ---------------------------------------------------------------------------- #

@test "error message when called without .flox directory" {
  run dot_flox_exists;
  assert_failure;
  run "$FLOX_CLI" delete;
  assert_failure;
  assert_output --partial "No environment found in \"$(pwd -P)\"";
}
