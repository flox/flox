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

setup_file() {
  export FLOX_FEATURES_ENV=rust
}

@test "'flox list' lists packages of environment in the current dirl; fails if no env found" {
  run "$FLOX_CLI" list
  assert_failure
  assert_output "No matching environments found"
}

@test "'flox list' lists packages of environment in the current dir; No package" {
  "$FLOX_CLI" init
  run "$FLOX_CLI" list
  assert_success
  assert_output --regexp ".*Packages in test:"
}

@test "'flox list' lists packages of environment in the current dir; One package from nixpkgs" {
  "$FLOX_CLI" init
  "$FLOX_CLI" install hello

  run "$FLOX_CLI" list
  assert_success
  assert_output --regexp - <<EOF
.*
Packages in test:
stable.hello
EOF
}

@test "'flox list' lists packages of environment in the current dir; matching names" {
  "$FLOX_CLI" init -e not-test
  "$FLOX_CLI" install -e not-test hello

  run "$FLOX_CLI" list -e not-test
  assert_success
  assert_output --regexp - <<EOF
.*
Packages in not-test:
stable.hello
EOF
}


@test "'flox list' lists packages of environment in the current dir; no name" {
  "$FLOX_CLI" init -e not-test
  "$FLOX_CLI" install -e not-test hello

  run "$FLOX_CLI" list
  assert_success
  assert_output --regexp - <<EOF
.*
Packages in not-test:
stable.hello
EOF
}

@test "'flox list' lists packages of environment in the current dir; no matching name" {
  "$FLOX_CLI" init -e not-test
  "$FLOX_CLI" install -e not-test hello

  run "$FLOX_CLI" list -e test
  assert_failure
  assert_output "No matching environments found"
}
