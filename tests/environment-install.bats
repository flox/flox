#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test rust impl of `flox install`
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

# without specifying a name should install to an environment found in the user's current directory.
@test "i2.a: install outside of shell (option1)" {
  skip "Environment defaults handled in another phase"
}

@test "flox install allows -r for installing to a specific remote environment name, creating a new generation." {
  skip "remote environments handled in another phase"
}

@test "'flox install' displays confirmation message" {
  "$FLOX_CLI" init;
  run "$FLOX_CLI" install hello;
  assert_success;
  assert_output --partial "✅ 'hello' installed to environment";
}

@test "'flox install' edits manifest" {
  "$FLOX_CLI" init;
  run "$FLOX_CLI" install hello;
  assert_success;
  run grep "hello = {}" "$PROJECT_DIR/.flox/env/manifest.toml";
  assert_success;
}

@test "uninstall confirmation message" {
  "$FLOX_CLI" init
  run "$FLOX_CLI" install hello
  assert_success
  assert_output --partial "✅ 'hello' installed to environment"

  run "$FLOX_CLI" uninstall hello
  assert_success
  # Note that there's TWO spaces between the emoji and the package name
  assert_output --partial "🗑️  'hello' uninstalled from environment"
}

@test "'flox uninstall' edits manifest" {
  "$FLOX_CLI" init;
  run "$FLOX_CLI" install hello;
  assert_success;
  run "$FLOX_CLI" uninstall hello;
  run grep "^hello = {}" "$PROJECT_DIR/.flox/env/manifest.toml";
  assert_failure;
}

@test "i5: download package when install command runs" {
  skip "Don't know how to test, check out-link created?"
}

@test "i6: install on a pushed environment stages locally" {
  skip "remote environments handled in another phase"
}
