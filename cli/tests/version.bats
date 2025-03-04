#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test the `flox --version` command.
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# bats file_tags=version

# ---------------------------------------------------------------------------- #

# Helpers for project based tests.

project_setup() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/project-${BATS_TEST_NUMBER?}"
  export PROJECT_NAME="${PROJECT_DIR##*/}"

  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" > /dev/null || return

  "$FLOX_BIN" init -d "$PROJECT_DIR"
}

project_teardown() {
  popd > /dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
}

# ---------------------------------------------------------------------------- #

setup() {
  common_test_setup
  setup_isolated_flox
  project_setup
}

teardown() {
  project_teardown
  common_test_teardown
}

function is_local_dev() {
  [[ "${_FLOX_LOCAL_DEV}" == "1" ]]
}

function skip_if_local_dev() {
  if is_local_dev; then
    skip "Skipping test in local development mode"
  fi
}

function skip_if_not_local_dev() {
  if ! is_local_dev; then
    skip "Skipping test not in local development mode"
  fi
}

# We can't easily or safely predict the buildtime version so assert that the two
# different formats never appear at the same time. When running in CI remote
# builders it will fallback to "0.0.0-dirty".
MOCK_RUNTIME_VERSION="1.2.3"
REGEX_BUILDTIME_VERSION='^([0-9]+\.[0-9]+\.[0-9]+-g.+|0.0.0-dirty)$'

function assert_runtime_version() {
  assert_output "$MOCK_RUNTIME_VERSION"
  refute_output --regexp "$REGEX_BUILDTIME_VERSION"
}

function assert_buildtime_version() {
  assert_output --regexp "$REGEX_BUILDTIME_VERSION"
  refute_output --partial "$MOCK_RUNTIME_VERSION"
}

@test "version: binary accepts runtime version from wrapper derivation" {
  skip_if_not_local_dev
  FLOX_VERSION="$MOCK_RUNTIME_VERSION" run "$FLOX_BIN" --version
  assert_success
  assert_runtime_version
}

@test "version: wrapper derivation ignores runtime FLOX_VERSION" {
  skip_if_local_dev
  FLOX_VERSION="$MOCK_RUNTIME_VERSION" run "$FLOX_BIN" --version
  assert_success
  assert_buildtime_version
}

@test "version: doesn't leak FLOX_VERSION into activation" {
  project_setup
  FLOX_VERSION="$MOCK_RUNTIME_VERSION" run "$FLOX_BIN" activate -- printenv FLOX_VERSION
  assert_failure
  assert_output ""
}
