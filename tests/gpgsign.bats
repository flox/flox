#!/usr/bin/env bats

load test_support.bash

setup_file() {
  common_setup;
}

@test "create environment with git global gpgsign set" {
  TEST_CASE_ENVIRONMENT=$(echo $RANDOM | md5sum | head -c 20; echo)

  run git config --global commit.gpgsign true;
  assert_success

  run $FLOX_CLI create -e $TEST_CASE_ENVIRONMENT;
  assert_success

  run $FLOX_CLI install -e $TEST_CASE_ENVIRONMENT cowsay;
  assert_success

  run $FLOX_CLI activate -e $TEST_CASE_ENVIRONMENT -- sh -c 'cowsay "Signature set in Global Config" >&2'
  assert_success

  run git config --global --unset commit.gpgsign;
  assert_success
}

@test "create environment with git user gpgsign set" {
  TEST_CASE_ENVIRONMENT=$(echo $RANDOM | md5sum | head -c 20; echo)

  run git config commit.gpgsign true;
  assert_success

  run $FLOX_CLI create -e $TEST_CASE_ENVIRONMENT;
  assert_success

  run $FLOX_CLI install -e $TEST_CASE_ENVIRONMENT cowsay;
  assert_success

  run $FLOX_CLI activate -e $TEST_CASE_ENVIRONMENT -- sh -c 'cowsay "Signature set in User Config" >&2'
  assert_success

  run git config --unset commit.gpgsign;
  assert_success
}

# vim:ts=4:noet:syntax=bash
