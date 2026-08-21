#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Tests for DEV-268: the 'auth_notifications' config key, which quiets the
# advisory "not logged in to FloxHub" reminder printed after a command
# completes.
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# bats file_tags=auth

# ---------------------------------------------------------------------------- #

setup_file() {
  common_file_setup
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.yaml"
}

teardown_file() {
  unset _FLOX_USE_CATALOG_MOCK
  common_file_teardown
}

# ---------------------------------------------------------------------------- #

project_setup() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/project-${BATS_TEST_NUMBER?}"
  export PROJECT_NAME="${PROJECT_DIR##*/}"
  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" > /dev/null || return
  setup_isolated_flox
  unset FLOX_FLOXHUB_TOKEN
  "$FLOX_BIN" auth logout
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

# bats test_tags=auth,auth:notifications
@test "logged-out reminder prints by default" {
  # 'flox config' is a purely local command that succeeds in a fresh
  # isolated setup ('flox envs' would fail with "no registry found").
  run "$FLOX_BIN" config
  assert_success
  assert_output --partial "not logged in to FloxHub"
}

# bats test_tags=auth,auth:notifications
@test "logged-out reminder is quieted via config file" {
  mkdir -p "$FLOX_CONFIG_DIR"
  echo 'auth_notifications = false' >> "$FLOX_CONFIG_DIR/flox.toml"

  run "$FLOX_BIN" config
  assert_success
  refute_output --partial "not logged in to FloxHub"
}

# ---------------------------------------------------------------------------- #
