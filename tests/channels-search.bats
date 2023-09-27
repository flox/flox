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
  common_test_setup;
  project_setup;
}
teardown() {
  project_teardown;
  common_test_teardown;
}

setup_file() {
  export FLOX_FEATURES_CHANNELS=rust;
}

# ---------------------------------------------------------------------------- #

@test "'flox search' can be called successfully" {
  run $FLOX_CLI search hello;
  assert_success;
}


# ---------------------------------------------------------------------------- #

@test "'flox search' errors with no search term" {
  run $FLOX_CLI search;
  assert_failure;
}


# ---------------------------------------------------------------------------- #

@test "'flox search' displays results" {
  n_lines=$($FLOX_CLI search hello | wc -l);
  assert [[ n_lines -gt 0 ]];
}
