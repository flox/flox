#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test of rust impl of 'flox list'
#
# Tests are tentative, missing spec!
#
# ---------------------------------------------------------------------------- #

load test_support.bash

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

@test "'flox list' lists packages of environment in the current dir; fails if no env found" {
  run "$FLOX_CLI" list;
  assert_failure;
}

@test "'flox list' lists packages of environment in the current dir; No package" {
  "$FLOX_CLI" init
  run "$FLOX_CLI" list
  assert_success
}

@test "'flox list' lists packages of environment in the current dir; One package from nixpkgs" {
  "$FLOX_CLI" init
  "$FLOX_CLI" install hello

  run "$FLOX_CLI" list
  assert_success
  assert_output --regexp - <<EOF
hello
EOF
}
