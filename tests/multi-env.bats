#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test the `FLOX_ENV' variable set by `flox activate' is set appropriately when
# multiple environments are indicated on the CLI.
#
# ---------------------------------------------------------------------------- #

load test_support.bash;


# ---------------------------------------------------------------------------- #

destroy_envs() {
  "$FLOX_CLI" destroy -e "${TEST_ENVIRONMENT}_multi_1" --origin -f||:;
  "$FLOX_CLI" destroy -e "${TEST_ENVIRONMENT}_multi_1" --origin -f||:;
}

setup_file() {
  common_setup;
  destroy_envs;
}

teardown_file() {
  destroy_envs;
}


# ---------------------------------------------------------------------------- #

@test "init flox ${TEST_ENVIRONMENT}_multi_{1,2}" {
  run "$FLOX_CLI" create -e "${TEST_ENVIRONMENT}_multi_1";
  assert_success;
  run "$FLOX_CLI" install -e "${TEST_ENVIRONMENT}_multi_1" "$FLOX_PACKAGE";
  assert_success;

  run "$FLOX_CLI" create -e "${TEST_ENVIRONMENT}_multi_2";
  assert_success;
  run "$FLOX_CLI" install -e "${TEST_ENVIRONMENT}_multi_2" "$FLOX_PACKAGE";
  assert_success;
}


# ---------------------------------------------------------------------------- #

@test "flox activate on multiple environments" {
  #shellcheck disable=SC2016
  run "$FLOX_CLI" activate                                               \
      -e "${TEST_ENVIRONMENT}_multi_1" -e "${TEST_ENVIRONMENT}_multi_2"  \
      -- bash -c 'echo "FLOX_ENV: $FLOX_ENV"';
  assert_success;
  # FLOX_ENV should be set to the first argument
  assert_output --partial "FLOX_ENV: ${FLOX_ENVIRONMENTS}/local/${NIX_SYSTEM}.${TEST_ENVIRONMENT}_multi_1";
}


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
