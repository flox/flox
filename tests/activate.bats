#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test the `flox activate' subcommand.
# We are especially interested in ensuring that the activation script works
# with most common shells, since that routine will be executed using the users
# running shell.
#
#
# ---------------------------------------------------------------------------- #

load test_support.bash;


# ---------------------------------------------------------------------------- #

destroy_envs() {
  "$FLOX_CLI" destroy -e "$TEST_ENVIRONMENT" --origin -f||:;
}

setup_file() {
  common_setup;
  export TEST_ENVIRONMENT='_testing_activate'
  destroy_envs;
  "$FLOX_CLI" create -e "$TEST_ENVIRONMENT";
  "$FLOX_CLI" install -e "$TEST_ENVIRONMENT" hello cowsay;

  # Run by various shells to test that `flox activate ... -- ...;' works.
  _inline_cmd="$FLOX_CLI activate -e '$TEST_ENVIRONMENT'";
  _inline_cmd="$_inline_cmd -- sh -c 'hello|cowsay'";
  export _inline_cmd;

  # Run by various shells to test that `eval "$( flox activate ...; )";' works.
  _eval_cmd="eval \"\$( $FLOX_CLI activate -e '$TEST_ENVIRONMENT'; )\"";
  _eval_cmd="$_eval_cmd; hello|cowsay;";
  export _eval_cmd;
}

teardown_file() {
  destroy_envs;
}

# ---------------------------------------------------------------------------- #

@test "'flox activate' can invoke hello and cowsay" {
  run sh -c "$_inline_cmd";
  assert_success;
  assert_output --partial - < tests/hello-cowsay.out;
}


# ---------------------------------------------------------------------------- #

@test "'flox activate' with eval can invoke hello and cowsay" {
  run sh -c "$_eval_cmd";
  assert_success;
  assert_output --partial - < tests/hello-cowsay.out;
}


# ---------------------------------------------------------------------------- #

@test "'flox activate' works with 'bash'" {
  run bash -c "$_inline_cmd";
  assert_success;
  assert_output --partial - < tests/hello-cowsay.out;
}


# ---------------------------------------------------------------------------- #

@test "'flox activate' with eval works with 'bash'" {
  run bash -c "$_eval_cmd";
  assert_success;
  assert_output --partial - < tests/hello-cowsay.out;
}


# ---------------------------------------------------------------------------- #

@test "'flox activate' works with 'dash'" {
  run dash -c "$_inline_cmd";
  assert_success;
  assert_output --partial - < tests/hello-cowsay.out;
}


# ---------------------------------------------------------------------------- #

@test "'flox activate' with eval works with 'dash'" {
  run dash -c "$_eval_cmd";
  assert_success;
  assert_output --partial - < tests/hello-cowsay.out;
}


# ---------------------------------------------------------------------------- #

@test "'flox activate' works with 'zsh'" {
  run zsh -c "$_inline_cmd";
  assert_success;
  assert_output --partial - < tests/hello-cowsay.out;
}


# ---------------------------------------------------------------------------- #

@test "'flox activate' with eval works with 'zsh'" {
  run zsh -c "$_eval_cmd";
  assert_success;
  assert_output --partial - < tests/hello-cowsay.out;
}


# ---------------------------------------------------------------------------- #

@test "'flox activate' accepts '-s,--system' options" {
  run "$FLOX_CLI" activate -e "$TEST_ENVIRONMENT" --system "$NIX_SYSTEM"  \
                           -- sh -c ':'
  assert_success
  run "$FLOX_CLI" activate -e "$TEST_ENVIRONMENT" -s "$NIX_SYSTEM"  \
                           -- sh -c ':'
  assert_success
}


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
