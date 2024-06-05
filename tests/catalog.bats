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
  export FLOX_FEATURES_USE_CATALOG=true
  if [ -z "${TESTING_FLOX_CATALOG_URL:-}" ]; then
    skip "TESTING_FLOX_CATALOG_URL is not set"
  fi
  export FLOX_CATALOG_URL="$TESTING_FLOX_CATALOG_URL"
}

teardown_file() {
  unset FLOX_FEATURES_USE_CATALOG
  unset FLOX_CATALOG_URL
  common_file_teardown
}

# ---------------------------------------------------------------------------- #

@test "'flox search' works with catalog server" {

  run "$FLOX_BIN" search hello -vvv
  assert_output --partial "using catalog client for search"
  assert_output --partial "hello"
  assert_output --partial "A program that produces a familiar, friendly greeting"
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
  assert_output --partial "using catalog client to lock"

  run "$FLOX_BIN" activate -- hello
  assert_success
  assert_output --partial "Hello, world!"

  "$FLOX_BIN" delete
}

# bats test_tags=upgrade:catalog
@test "'flox upgrade' works with catalog server" {
  "$FLOX_BIN" init

  _FLOX_USE_CATALOG_MOCK="$TESTS_DIR/catalog_responses/hello_resolution_old.json" \
    "$FLOX_BIN" install -i hello_install_id hello

  run "$FLOX_BIN" list
  assert_success
  assert_line "hello_install_id: hello (old_version)"

  _FLOX_USE_CATALOG_MOCK="$TESTS_DIR/catalog_responses/hello.json" \
    run "$FLOX_BIN" upgrade -vvv
  assert_success
  assert_output --partial "using catalog client to upgrade"
  assert_output --partial "Upgraded 'hello_install_id'"

  run "$FLOX_BIN" list
  assert_success
  assert_line "hello_install_id: hello (2.12.1)"

  "$FLOX_BIN" delete
}
