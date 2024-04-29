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
  assert_output --partial "hello - hello@2.12.1"
}
