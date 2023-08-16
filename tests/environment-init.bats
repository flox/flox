#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test flox init
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

@test "c2: flox init without a name should create an environment named the same as the directory the user is in" {

  run "$FLOX_CLI" init
  assert_success

  run "$FLOX_CLI" envs
  assert_success
  assert_output "test"
}

@test "c2: If the user is in ~ the environment should be called 'default'." {

  skip "Can't mock user / home dir"

  export HOME="$PROJECT_DIR"

  run "$FLOX_CLI" init
  assert_success

  run "$FLOX_CLI" envs
  assert_success
  assert_output "default"

}

@test "c4: custom name option 1: flox init accepts -e for a user defined name" {
  run "$FLOX_CLI" init -e "other-test"
  assert_success

  run "$FLOX_CLI" envs
  assert_success
  assert_output --partial "other-test"
}

@test "c6: a single directory for state" {
  run "$FLOX_CLI" init
  assert_success

  run ls -A
  assert_output ".flox"
}

@test "c7: confirmation with tips" {
  run "$FLOX_CLI" init
  assert_success

  assert_output - <<EOF
✨ created environment test ($NIX_SYSTEM)

Enter the environment with "flox activate"
Search and install packages with "flox search {packagename}" and "flox install {packagename}"
EOF

}

@test "c8: names don't conflict with flox hub: when naming with flox init -e do not allow '/'" {
  run "$FLOX_CLI" init -e "owner/name"
  assert_failure
}

@test "c8: names don't conflict with flox hub: when naming with flox init -e do not allow ' ' (space)" {
  run "$FLOX_CLI" init -e "na me"
  assert_failure
}
