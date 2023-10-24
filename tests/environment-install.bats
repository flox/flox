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

setup_file() {
  export FLOX_FEATURES_ENV=rust
}

# without specifying a name should install to an environment found in the user's current directory.
@test "i2.a: install outside of shell (option1)" {
  skip "Environment defaults handled in another phase"
}

@test "flox install allows -r for installing to a specific remote environment name, creating a new generation." {
  skip "remote environments handled in another phase"
}

@test "i?: install confirmation message" {
  "$FLOX_CLI" init
  run "$FLOX_CLI" install hello
  assert_success
  assert_output --partial "✅ Installed 'hello' into 'test' environment."
}

@test "uninstall confirmation message" {
  "$FLOX_CLI" init
  run "$FLOX_CLI" install hello
  assert_success
  assert_output --partial "✅ Installed 'hello' into 'test' environment."

  run "$FLOX_CLI" uninstall hello
  assert_success
  assert_output --partial "🗑️ Uninstalled 'hello' from 'test' environment."
}

@test "i?: warning message if package is already installed {
  skip "our current editing of Nix expressions doesn't detect already installed packages."
  run "$FLOX_CLI" install hello # install once
  run "$FLOX_CLI" install hello # try install again
  assert_success
  assert_output --partial "...already installed..."
}

@test "i5: download package when install command runs" {
  skip "Don't know how to test, check out-link created?"
}

@test "i6: install on a pushed environment stages locally" {
  skip "remote environments handled in another phase"
}
