#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test the rules engine
# bats file_tags=rules
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# ---------------------------------------------------------------------------- #

# Helpers for project based tests.

project_setup() {
  export PROJECT_NAME="test"
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/$PROJECT_NAME"
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
  project_setup
}

teardown() {
  project_teardown
  common_test_teardown
}


# ---------------------------------------------------------------------------- #

@test "searches unfree packages" {
  # elasticsearch uses the Elastic License 2.0, which is unfree
  # Command broken over two lines only for line length
  cmd="$FLOX_BIN search elasticsearch --json | jq -r -c '.[] | .relPath[0]'"
  cmd="$cmd | sed -n '/^elasticsearch$/p'"
  run sh -c "$cmd"
  assert_success
  assert_equal "${#lines[@]}" 1
}
