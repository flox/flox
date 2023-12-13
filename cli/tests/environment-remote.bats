#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test rust impl of `flox install`
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# ---------------------------------------------------------------------------- #

# Helpers for project based tests.

project_setup() {
  export PROJECT_NAME="test";
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/$PROJECT_NAME"

  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" >/dev/null || return

}

# todo: use method pulled out in #620
floxhub_setup() {
  export FLOX_FLOXHUB_TOKEN=flox_testOAuthToken
  export FLOX_FLOXHUB_PATH="$BATS_TEST_TMPDIR/floxhub"
  export OWNER="owner"
  export FLOXHUB_FLOXMETA_DIR="$FLOX_FLOXHUB_PATH/$OWNER/floxmeta"

  mkdir -p "$FLOX_FLOXHUB_PATH"
  mkdir -p "$FLOXHUB_FLOXMETA_DIR"
  git -C "$FLOXHUB_FLOXMETA_DIR" init --bare

  export __FLOX_FLOXHUB_URL="file://$FLOX_FLOXHUB_PATH"

}


# tests should not share the same floxmeta repo.
# we also want to simulate different machines.
#
# floxmeta_setup <machine_name>
floxmeta_setup() {
  mkdir -p "$FLOXHUB_FLOXMETA_DIR/${1}"
  export FLOX_DATA_DIR="$BATS_TEST_TMPDIR/${1}"
}

project_teardown() {
  popd >/dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
}

setup() {
  common_test_setup
  project_setup
  floxhub_setup
  home_setup test;
}

teardown() {
  project_teardown
  common_test_teardown
}

# ---------------------------------------------------------------------------- #

function make_empty_remote_env() {
  mkdir local
  pushd local
  # init path environment and push to remote
  "$FLOX_BIN" init --name test
  "$FLOX_BIN" push --owner "$OWNER"
  # "$FLOX_BIN" delete -f # add after #621 lands
  popd
  rm -rf local
}

# ---------------------------------------------------------------------------- #

# bats test_tags=hermetic,remote,remote:hermetic
@test "r0: listing a remote environment does not create (visible) local files" {
  make_empty_remote_env

  run --separate-stderr "$FLOX_BIN" list --remote "$OWNER/test"
  assert_success
  assert_output ""

  run ls -lA .
  assert_success
  assert_output "total 0"
}


# bats test_tags=install,remote,remote:install
@test "m1: install a package to a remote environment" {
  make_empty_remote_env

  run "$FLOX_BIN" install hello --remote "$OWNER/test"
  assert_success
  assert_output --partial "environment $OWNER/test (remote)" # managed env output


  run --separate-stderr "$FLOX_BIN" list --remote "$OWNER/test"
  assert_success
  assert_output "hello"
}


# bats test_tags=uninstall,remote,remote:uninstall
@test "m2: uninstall a package from a managed environment" {
  make_empty_remote_env

  "$FLOX_BIN" install emacs vim --remote "$OWNER/test"

  run "$FLOX_BIN" uninstall vim --remote "$OWNER/test"
  assert_success

  run --separate-stderr "$FLOX_BIN" list --remote "$OWNER/test"
  assert_success
  assert_output "emacs"
}

# bats test_tags=edit,remote,remote:edit
@test "m3: edit a package from a managed environment" {
  make_empty_remote_env

  TMP_MANIFEST_PATH="$BATS_TEST_TMPDIR/manifest.toml"

  cat << "EOF" >> "$TMP_MANIFEST_PATH"
[install]
hello = {}
EOF

  run "$FLOX_BIN" edit -f "$TMP_MANIFEST_PATH" --remote "$OWNER/test"
  assert_success
  assert_output --partial "✅ environment successfully edited"

  run --separate-stderr "$FLOX_BIN" list --remote "$OWNER/test"
  assert_success
  assert_output "hello"
}

# ---------------------------------------------------------------------------- #
