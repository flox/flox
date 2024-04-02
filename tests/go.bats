#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test if Go works with flox.
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# bats file_tags=end2end,go

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

@test "'flox init' sets up a local working Go module environment" {
  cp -r "$TESTS_DIR"/go/single-dependency/module/* "$PROJECT_DIR/"

  run "$FLOX_BIN" init --auto-setup
  assert_success
  assert_line --partial "'go' installed"

  run "$FLOX_BIN" activate -- go version
  assert_success
  assert_line --partial "go version go1."
}

# ---------------------------------------------------------------------------- #

@test "'flox init' sets up a local working Go workspace environment" {
  GO_VERSION_COMMAND="go version"
  cp -r "$TESTS_DIR"/go/single-dependency/workspace/* "$PROJECT_DIR/"

  run "$FLOX_BIN" init --auto-setup
  assert_success
  assert_line --partial "'go' installed"

  run "$FLOX_BIN" activate -- go version
  assert_success
  assert_line --partial "go version go1."
}
