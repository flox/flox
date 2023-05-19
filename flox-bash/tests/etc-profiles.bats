#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test basic usage of `etc/profile' script usage with `flox create' and
# `flox activate'.
#
# Notably ensure that things like `pkg-config' "just work" out of the box.
#
# ---------------------------------------------------------------------------- #

bats_load_library bats-support;
bats_load_library bats-assert;
bats_require_minimum_version 1.5.0;

load test_support.bash;

kill_envs() {
  "$FLOX_CLI" destroy -e "${TEST_ENVIRONMENT}1" --origin -f||:;
  "$FLOX_CLI" destroy -e "${TEST_ENVIRONMENT}2" --origin -f||:;
  "$FLOX_CLI" destroy -e "${TEST_ENVIRONMENT}3" --origin -f||:;
}

setup_file() {
  common_setup;
  kill_envs;
}

teardown_file() {
  kill_envs;
}


# ---------------------------------------------------------------------------- #

@test "default 'flox create' env can locate python3 pkg-config resources" {
  run "$FLOX_CLI" create -e "${TEST_ENVIRONMENT}1";
  assert_success;
  run "$FLOX_CLI" install -e "${TEST_ENVIRONMENT}1" python3 pkg-config libkrb5;
  assert_success;
  # `pkg-config' should be able to locate `python3' files.
  run "$FLOX_CLI" activate -e "${TEST_ENVIRONMENT}1" -- pkg-config --list-all;
  assert_success;
  assert_output --partial python3;

  # TODO: install all outputs by default so this works
  # assert_output --regexp '^krb5 +'
}


# ---------------------------------------------------------------------------- #

@test "flox create -P" {
  run "$FLOX_CLI" create -P -e "${TEST_ENVIRONMENT}2";
  assert_success;
  run "$FLOX_CLI" list -e "${TEST_ENVIRONMENT}2";
  refute_output --partial 'github:flox/etc-profiles';
}


# ---------------------------------------------------------------------------- #

@test "flox create --no-profiles" {
  run "$FLOX_CLI" create --no-profiles -e "${TEST_ENVIRONMENT}3";
  assert_success;
  run "$FLOX_CLI" list -e "${TEST_ENVIRONMENT}3";
  refute_output --partial 'github:flox/etc-profiles';
}


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
