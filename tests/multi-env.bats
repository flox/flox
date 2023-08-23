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

delete_envs() {
  deleteEnvForce "${TEST_ENVIRONMENT}-1";
  deleteEnvForce "${TEST_ENVIRONMENT}-2";
}

setup_file() {
  common_file_setup;
  hello_pkg_setup;
  deleteEnvForce "${TEST_ENVIRONMENT}-1";
  deleteEnvForce "${TEST_ENVIRONMENT}-2";
  $FLOX_CLI create  -e "${TEST_ENVIRONMENT}-1";
  $FLOX_CLI install -e "${TEST_ENVIRONMENT}-1" "$HELLO_PACKAGE";
  $FLOX_CLI create  -e "${TEST_ENVIRONMENT}-2";
  $FLOX_CLI install -e "${TEST_ENVIRONMENT}-2" "$HELLO_PACKAGE";
}

teardown_file() {
  if [[ -z "${FLOX_TEST_KEEP_TMP:-}" ]]; then
    deleteEnvForce "${TEST_ENVIRONMENT}-1";
    deleteEnvForce "${TEST_ENVIRONMENT}-2";
  fi
}


# ---------------------------------------------------------------------------- #

@test "flox activate on multiple environments" {
  #shellcheck disable=SC2016
  run "$FLOX_CLI" activate                                   \
      -e "${TEST_ENVIRONMENT}-1" -e "${TEST_ENVIRONMENT}-2"  \
      -- bash -c 'test -d "$FLOX_ENV" && echo "FLOX_ENV: $FLOX_ENV"';
  assert_success;
  # FLOX_ENV should be set to the first argument.  Note that the username
  # part of the path can be either "local" or "floxtest" based on whether
  # the flox-pushpull.bats test has yet to be run, in which case "local"
  # will be a symlink pointing to "floxtest".
  if [ -L "${FLOX_ENVIRONMENTS}/local" ]; then
    assert_output --partial "FLOX_ENV: ${FLOX_ENVIRONMENTS}/floxtest/${NIX_SYSTEM}.${TEST_ENVIRONMENT}-1";
  else
    assert_output --partial "FLOX_ENV: ${FLOX_ENVIRONMENTS}/local/${NIX_SYSTEM}.${TEST_ENVIRONMENT}-1";
  fi
}


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
