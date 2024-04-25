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
}

# ---------------------------------------------------------------------------- #


setup() {
  common_test_setup
  export FLOX_FEATURES_USE_CATALOG=true
  export FLOX_CATALOG_URL="https://flox-catalog.flox.dev"
}

teardown() {
  unset FLOX_FEATURES_USE_CATALOG
  unset FLOX_CATALOG_URL
  common_test_teardown
}

# ---------------------------------------------------------------------------- #

@test "'flox search' works with catalog server" {
  run "$FLOX_BIN" search hello
  assert_output --partial "hello"
  assert_output --partial "A program that produces a familiar, friendly greeting"
}

@test "'flox show' works with catalog server" {
  run "$FLOX_BIN" show hello
  assert_output --partial "hello - hello@2.12.1"
}
