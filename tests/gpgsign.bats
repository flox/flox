#!/usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Tests that `flox' can operate on `git' repositories with and without GPG
# signing keys.
#
# ---------------------------------------------------------------------------- #

load test_support.bash;

# bats file_tags=git, gpg, security


# ---------------------------------------------------------------------------- #

setup_file() { common_file_setup test; }
setup()      { home_setup test; cd "$FLOX_TEST_HOME"||return; }
teardown()   { cd "$BATS_RUN_TMPDIR"||return; }

# ---------------------------------------------------------------------------- #

@test "create environment with git global gpgsign set" {
  run git config --global commit.gpgsign true;
  assert_success;

  run $FLOX_CLI create -e "${TEST_ENVIRONMENT}1";
  assert_success;

  run $FLOX_CLI install -e "${TEST_ENVIRONMENT}1" cowsay;
  assert_success;

  run $FLOX_CLI activate -e "${TEST_ENVIRONMENT}1" --     \
    sh -c 'cowsay "Signature set in Global Config" >&2';
  assert_success;

  run git config --global --unset commit.gpgsign;
  assert_success;
}


# ---------------------------------------------------------------------------- #

@test "create environment with git user gpgsign set" {
  run git init;
  assert_success;

  run git config commit.gpgsign true;
  assert_success;

  run $FLOX_CLI create -e "${TEST_ENVIRONMENT}2";
  assert_success;

  run $FLOX_CLI install -e "${TEST_ENVIRONMENT}2" cowsay;
  assert_success;

  run $FLOX_CLI activate -e "${TEST_ENVIRONMENT}2" --   \
    sh -c 'cowsay "Signature set in User Config" >&2';
  assert_success;

  run git config --unset commit.gpgsign;
  assert_success;
}


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
# vim:ts=4:noet:syntax=bash
