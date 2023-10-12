#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test of rust impl of 'flox show'
#
# bats file_tags=search,show
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

@test "'flox show' can be called at all" {
  run "$FLOX_CLI" show hello;
  assert_success;
}


# ---------------------------------------------------------------------------- #

@test "'flox show' accepts specific input" {
  run "$FLOX_CLI" show nixpkgs-flox:hello;
  assert_success;
  # TODO: better testing once the formatting is implemented
}

# ---------------------------------------------------------------------------- #

@test "'flox show' accepts search output without separator" {
  run "$FLOX_CLI" search hello;
  assert_success;
  first_result="${lines[0]%% *}";
  run "$FLOX_CLI" show "$first_result";
  assert_success;
}


# ---------------------------------------------------------------------------- #

@test "'flox show' accepts search output with separator" {
  run "$FLOX_CLI" search nixpkgs-flox:hello;
  assert_success;
  first_result="${lines[0]%% *}";
  run "$FLOX_CLI" show "$first_result";
  assert_success;
}
