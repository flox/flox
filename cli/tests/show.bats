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
# Note in this file, these aren't added to setup() and teardown()

project_setup() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/test"
  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" >/dev/null || return
  run "$FLOX_BIN" init
  assert_success
  unset output
  export LOCKFILE_PATH="$PROJECT_DIR/.flox/env/manifest.lock"
}

project_teardown() {
  popd >/dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
  unset LOCKFILE_PATH
}

# ---------------------------------------------------------------------------- #

setup() {
  common_test_setup
  setup_isolated_flox
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.json"
}

teardown() {
  common_test_teardown
}

setup_file() {
  :
}

# ---------------------------------------------------------------------------- #

@test "'flox show' can be called at all" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/show/hello.json"
  run "$FLOX_BIN" show hello
  assert_success
}

# ---------------------------------------------------------------------------- #

@test "'flox show' accepts search output without separator" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA//search/hello.json"
  run "$FLOX_BIN" search hello
  assert_success
  first_result="${lines[0]%% *}"
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/show/hello.json"
  run "$FLOX_BIN" show "$first_result"
  assert_success
}

# ---------------------------------------------------------------------------- #

@test "'flox show' - hello" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/show/hello.json"
  run "$FLOX_BIN" show hello
  assert_success
  assert_equal "${lines[0]}" "hello - Program that produces a familiar, friendly greeting"
  assert_equal "${lines[1]}" "    hello@2.12.1"
  assert_equal "${lines[2]}" "    hello@2.12"
  assert_equal "${lines[3]}" "    hello@2.10"
}

# bats test_tags=python

# Check pkg-path is handled correctly
@test "'flox show' - python310Packages.flask" {
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/show/flask.json" \
    run "$FLOX_BIN" show python310Packages.flask
  assert_success
  # Ensure that the package and part of the description show up
  assert_output --partial 'python310Packages.flask - The'
}

# ---------------------------------------------------------------------------- #
