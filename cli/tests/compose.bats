
#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test environment composition
#
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# bats file_tags=compose

# ---------------------------------------------------------------------------- #

setup_file() {
  common_file_setup
}

setup() {
  common_test_setup
  home_setup test # Isolate $HOME for each test.
  setup_isolated_flox

  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.json"
}

teardown() {
  # fifo is in PROJECT_DIR and keeps watchdog running,
  # so cat_teardown_fifo must be run before wait_for_watchdogs and
  # project_teardown
  cat_teardown_fifo
  # Cleaning up the `BATS_TEST_TMPDIR` occasionally fails,
  # because of an 'env-registry.json' that gets concurrently written
  # by the watchdog as the activation terminates.
  if [ -n "${PROJECT_DIR:-}" ]; then
    # Not all tests call project_setup
    wait_for_watchdogs "$PROJECT_DIR" || return 1
    project_teardown
  fi
  common_test_teardown
}

# ---------------------------------------------------------------------------- #

# Helpers for project based tests.

project_setup_common() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/project-${BATS_TEST_NUMBER?}"
  export PROJECT_NAME="${PROJECT_DIR##*/}"

  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" >/dev/null || return

}

# setup with catalog
project_setup() {
  project_setup_common
  "$FLOX_BIN" init -d "$PROJECT_DIR"
}

project_teardown() {
  popd >/dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
  unset PROJECT_NAME
}

# ---------------------------------------------------------------------------- #

# bats test_tags=compose
@test "compose: feature flag works" {
  project_setup
  RUST_LOG=debug FLOX_FEATURES_COMPOSE=true run "$FLOX_BIN" activate -- true
  assert_output --partial "compose=true"
  RUST_LOG=debug FLOX_FEATURES_COMPOSE=false run "$FLOX_BIN" activate -- true
  assert_output --partial "compose=false"
}
