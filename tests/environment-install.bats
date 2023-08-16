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

  "$FLOX_CLI" init

  run "$FLOX_CLI" install hello
  assert_success

  run "$FLOX_CLI" list
  assert_success
  assert_output --regexp - <<EOF
.*
Packages in test:
stable.hello
EOF
}

@test "i3: flox install allows -e for explicit environment name;  If .flox does not exist, a .flox is created." {
  run ls .flox
  assert_failure

  run "$FLOX_CLI" install -e env hello
  assert_success

  run ls .flox
  assert_success
}

@test "i3: flox install allows -e for explicit environment name;  If the environment is not staged in FLOX_META, it is pulled" {
  skip "remote environments handled in another phase"
}

@test "i3: flox install allows -e for explicit environment name; If the environment exists in FLOX_META or locally, .flox is a link" {
  skip "remote environments handled in another phase"
}

@test "i4: If -e specifies an environment different than the one in .flox, an error is thrown" {
  "$FLOX_CLI" init -e env
  run "$FLOX_CLI" install -e not-env hello
  assert_failure
  assert_output "Env Not found"
}

@test "i4: confirmation message" {
  "$FLOX_CLI" init
  run "$FLOX_CLI" install hello
  assert_success
  assert_output "✅ Installed 'hello' package(s) into 'test' environment."
}

@test "i5: download package when install command runs" {
  skip "Don't know how to test, check out-link created?"
}

@test "i6: install on a pushed environment stages locally" {
  skip "remote environments handled in another phase"
}
