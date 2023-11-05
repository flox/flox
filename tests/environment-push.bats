#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test rust impl of `flox pull`
#
# ---------------------------------------------------------------------------- #

load test_support.bash
# bats file_tags=push

# ---------------------------------------------------------------------------- #

# Helpers for project based tests.

project_setup() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/test"
  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" >/dev/null || return
}

project_teardown() {
  popd >/dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
}

# ---------------------------------------------------------------------------- #

setup() {
  home_setup test;
  common_test_setup
  project_setup

  export FLOX_FLOXHUB_TOKEN=flox_testOAuthToken
  export __FLOX_FLOXHUB_URL="file://$BATS_TEST_TMPDIR/floxhub"
}
teardown() {
  unset __FLOX_FLOXHUB_URL
  project_teardown
  common_test_teardown
}



# simulate a dummy env update pushed to floxhub
function update_dummy_env() {
  OWNER="$1"; shift;
  ENV_NAME="$1"; shift;

  FLOXHUB_FLOXMETA_DIR="$BATS_TEST_TMPDIR/floxhub/$OWNER/floxmeta"

  pushd "$FLOXHUB_FLOXMETA_DIR" >/dev/null || return

  touch new_file
  git add .
  git commit -m "update"

  popd >/dev/null || return
}

# ---------------------------------------------------------------------------- #


# bats test_tags=push:h1
@test "l1: push login: running flox push before user has login metadata prompts the user to login" {

  unset FLOX_FLOXHUB_TOKEN; # logout, effectively

  run "$FLOX_CLI" init

  run "$FLOX_CLI" push --owner owner # dummy owner
  assert_failure
  assert_output --partial 'Please login to floxhub with `flox auth login`'
}
