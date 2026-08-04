#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test integration with an actual catalog server.
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# bats file_tags=end2end

# ---------------------------------------------------------------------------- #

setup_file() {
  common_file_setup
  export FLOX_CATALOG_URL="https://api.preview.flox.dev"
}

teardown_file() {
  unset FLOX_CATALOG_URL
  common_file_teardown
}

# ---------------------------------------------------------------------------- #

setup() {
  common_test_setup
  # These tests hit the real preview catalog anonymously; the suite-wide
  # dummy JWT would be rejected with 401.
  unset FLOX_FLOXHUB_TOKEN
  # Isolate each test in its own directory so a failure can never strand a
  # `.flox` in the shared bats cwd, where later prompt-hook tests would
  # discover it.
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/project-${BATS_TEST_NUMBER?}"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" > /dev/null || return
}

teardown() {
  popd > /dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
  common_test_teardown
}

@test "'flox search' works with catalog server" {

  run "$FLOX_BIN" search hello -vvv
  assert_output --partial "using catalog client for search"
  assert_output --partial "hello"
  assert_output --partial "a familiar, friendly greeting"
}

@test "'flox show' works with catalog server" {
  run "$FLOX_BIN" show hello -vvv
  assert_output --partial "using catalog client for show"
  assert_output --partial "hello@2.12.1"
}

@test "'flox install' and 'flox activate' work with catalog server" {
  "$FLOX_BIN" init

  run "$FLOX_BIN" install hello -vvv
  assert_success

  run "$FLOX_BIN" activate -- hello
  assert_success
  assert_output --partial "Hello, world!"

  "$FLOX_BIN" delete
}
