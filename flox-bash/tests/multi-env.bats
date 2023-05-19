#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test the `FLOX_ENV' variable set by `flox activate' is set appropriately when
# multiple environments are indicated on the CLI.
#
# ---------------------------------------------------------------------------- #

bats_load_library bats-support;
bats_load_library bats-assert;
bats_require_minimum_version 1.5.0;

load test_support.bash;


# ---------------------------------------------------------------------------- #

@test "init flox ${TEST_ENVIRONMENT}{1,2}" {
  run "$FLOX_CLI" create -e "${TEST_ENVIRONMENT}1";
  assert_success;
  run "$FLOX_CLI" install -e "${TEST_ENVIRONMENT}1" "$FLOX_PACKAGE";
  assert_success;

  run "$FLOX_CLI" create -e "${TEST_ENVIRONMENT}2";
  assert_success;
  run "$FLOX_CLI" install -e "${TEST_ENVIRONMENT}2" "$FLOX_PACKAGE";
  assert_success;
}


# ---------------------------------------------------------------------------- #

@test "flox activate on multiple environments" {
  #shellcheck disable=SC2016
  run "$FLOX_CLI" activate                                 \
      -e "${TEST_ENVIRONMENT}1" -e "${TEST_ENVIRONMENT}2"  \
      -- bash -c 'echo "FLOX_ENV: $FLOX_ENV"';
  assert_success;
  # FLOX_ENV should be set to the first argument
  assert_output --regexp "^FLOX_ENV: .*${TEST_ENVIRONMENT}1\$";
}


# ---------------------------------------------------------------------------- #

@test "teardown ${TEST_ENVIRONMENT}{1,2}" {
  run "$FLOX_CLI" destroy -e "${TEST_ENVIRONMENT}1" -f;
  assert_success;
  run "$FLOX_CLI" destroy -e "${TEST_ENVIRONMENT}2" -f;
  assert_success;
}


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
