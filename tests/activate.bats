#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test the `flox activate' subcommand.
# We are especially interested in ensuring that the activation script works
# with most common shells, since that routine will be executed using the users
# running shell.
#
# TODO: Test multiple versions of `bash'. Specifically v{3,4,5}.x
#
# ---------------------------------------------------------------------------- #

load test_support.bash;


# ---------------------------------------------------------------------------- #

destroy_envs() {
  "$FLOX_CLI" destroy -e "$TEST_ENVIRONMENT" --origin -f||:;
}

setup_file() {
  common_setup;
  TEST_ENVIRONMENT='_testing_activate'
  destroy_envs;
  "$FLOX_CLI" create -e "$TEST_ENVIRONMENT";
  "$FLOX_CLI" install -e "$TEST_ENVIRONMENT" hello cowsay;
}

teardown_file() {
  destroy_envs;
}


# ---------------------------------------------------------------------------- #

@test "'flox activate' can invoke hello and cowsay" {
  run "$FLOX_CLI" activate -e "$TEST_ENVIRONMENT" -- sh -c 'hello|cowsay;';
  assert_success;
  assert_output --partial - < tests/hello-cowsay.out;
}


# ---------------------------------------------------------------------------- #

@test "'flox activate' works with 'bash'" {
  run bash -c "$FLOX_CLI activate -e '$TEST_ENVIRONMENT' -- bash -c ':';";
  assert_success;
}


# ---------------------------------------------------------------------------- #

@test "'flox activate' works with 'dash'" {
  run bash -c "$FLOX_CLI activate -e '$TEST_ENVIRONMENT' -- dash -c ':';";
  assert_success;
}


# ---------------------------------------------------------------------------- #

@test "'flox activate' works with 'zsh'" {
  run bash -c "$FLOX_CLI activate -e '$TEST_ENVIRONMENT' -- zsh -c ':';";
  assert_success;
}


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
