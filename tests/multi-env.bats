#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test the `FLOX_ENV' variable set by `flox activate' is set appropriately when
# multiple environments are indicated on the CLI.
#
# ---------------------------------------------------------------------------- #

load test_support.bash;

# bats file_tags=activate

# ---------------------------------------------------------------------------- #

destroy_envs() {
  destroyEnvForce "${TEST_ENVIRONMENT}1";
  destroyEnvForce "${TEST_ENVIRONMENT}2";
}

setup_file() {
  common_file_setup;
  hello_pkg_setup;
  destroyEnvForce "${TEST_ENVIRONMENT}1";
  destroyEnvForce "${TEST_ENVIRONMENT}2";
  $FLOX_CLI create  -e "${TEST_ENVIRONMENT}1";
  $FLOX_CLI install -e "${TEST_ENVIRONMENT}1" "$HELLO_PACKAGE";
  $FLOX_CLI create  -e "${TEST_ENVIRONMENT}2";
  $FLOX_CLI install -e "${TEST_ENVIRONMENT}2" "$HELLO_PACKAGE";
}

teardown_file() {
  if [[ -z "${FLOX_TEST_KEEP_TMP:-}" ]]; then
    destroyEnvForce "${TEST_ENVIRONMENT}1";
    destroyEnvForce "${TEST_ENVIRONMENT}2";
  fi
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
