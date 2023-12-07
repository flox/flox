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


function make_dummy_floxmeta() {
  OWNER="$1"; shift;

  FLOXHUB_FLOXMETA_DIR="$BATS_TEST_TMPDIR/floxhub/$OWNER/floxmeta"

  mkdir -p "$FLOXHUB_FLOXMETA_DIR"
  pushd "$FLOXHUB_FLOXMETA_DIR"

  # todo: fake a real upstream env
  git init --initial-branch="dummy"

  git config user.name "test"
  git config user.email "test@email.address"

  touch "dummy"
  git add .
  git commit -m "initial commit"

  popd >/dev/null || return
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
@test "h1: push login: running flox push before user has login metadata prompts the user to login" {
  make_dummy_floxmeta "owner"

  unset FLOX_FLOXHUB_TOKEN; # logout, effectively

  run "$FLOX_BIN" config

  run "$FLOX_BIN" init

  run "$FLOX_BIN" push --owner owner # dummy owner
  assert_failure
  assert_output --partial 'Please login to floxhub with `flox auth login`'
}


# bats test_tags=push:h2
@test "h2: push login: running flox push before user has login metadata prompts the user to login" {
  make_dummy_floxmeta "owner"


  mkdir -p "machine_a"
  mkdir -p "machine_b"

  pushd "machine_a" >/dev/null || return
  run "$FLOX_BIN" init --name "test"
  run "$FLOX_BIN" install hello
  run "$FLOX_BIN" push --owner owner
  popd >/dev/null || return


  pushd "machine_b" >/dev/null || return
  run "$FLOX_BIN" pull --remote owner/test
  assert_success

  run "$FLOX_BIN" list
  assert_success
  assert_line "hello"

  popd >/dev/null || return
}


# bats test_tags=push:h5
@test "h5: unique upstream environments: if you attempt to flox push an environment with the same name but different provenance from upstream then the push should fail with a message." {
  make_dummy_floxmeta "owner"

  mkdir -p "machine_a"
  mkdir -p "machine_b"

  # Create an environment owner/test on machine_a and push it to floxhub
  pushd "machine_a" >/dev/null || return
  "$FLOX_BIN" init --name "test"
  "$FLOX_BIN" install vim
  "$FLOX_BIN" push --owner owner
  popd >/dev/null || return

  # Create an environment owner/test on machine_b and try to push it to floxhub
  # this should fail as an envrioment with the same name but different provenance already exists on floxhub
  pushd "machine_b" >/dev/null || return
  "$FLOX_BIN" init --name "test"
  "$FLOX_BIN" install emacs

  run "$FLOX_BIN" push --owner owner
  assert_failure
  assert_output --partial "upstream floxmeta branch diverged from local branch"
  popd >/dev/null || return
}


# bats test_tags=push:h6
@test "h6: force push upstream: adding -f option to flox push will force push an environment upstream even if an existing environment of the same name exists with different provenance." {
  make_dummy_floxmeta "owner"

  mkdir -p "machine_a" "machine_b" "machine_c"

  # Create an environment owner/test on machine_a and push it to floxhub
  pushd "machine_a" >/dev/null || return
  "$FLOX_BIN" init --name "test"
  "$FLOX_BIN" install vim
  "$FLOX_BIN" push --owner owner
  popd >/dev/null || return

  # Create an environment owner/test on machine_b and force-push it to floxhub
  pushd "machine_b" >/dev/null || return
  "$FLOX_BIN" init --name "test"
  "$FLOX_BIN" install emacs
  run "$FLOX_BIN" push --owner owner --force
  assert_success
  popd >/dev/null || return

  # Pull the environment owner/test on machine_c and check that it has the emacs package
  pushd "machine_c" >/dev/null || return
  "$FLOX_BIN" pull --remote owner/test
  run "$FLOX_BIN" list
  assert_success
  assert_line "emacs"
  popd >/dev/null || return
}
