#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test if node works with flox activate.
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# bats file_tags=end2end

# ---------------------------------------------------------------------------- #

setup_file() {
  common_file_setup
}

# ---------------------------------------------------------------------------- #

# Helpers for project based tests.

project_setup() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/project-${BATS_TEST_NUMBER?}"
  export PROJECT_NAME="${PROJECT_DIR##*/}"
  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" > /dev/null || return
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

@test "flox activate works with npm" {
  cp -r "$TESTS_DIR/node/single-dependency/common/." .
  cp -r "$TESTS_DIR/node/single-dependency/npm/." .
  run "$FLOX_BIN" init --auto-setup
  assert_output --partial "'nodejs' installed"
  run "$FLOX_BIN" activate -- npm run start
  assert_output --partial "86400000"
}

@test "flox activate works with yarn" {
  cp -r "$TESTS_DIR/node/single-dependency/common/." .
  cp -r "$TESTS_DIR/node/single-dependency/yarn/." .
  run "$FLOX_BIN" init --auto-setup
  assert_output --partial "'yarn' installed"
  refute_output "nodejs"
  run "$FLOX_BIN" activate -- yarn run start
  assert_output --partial "86400000"
}
