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
  export OWNER="owner";

  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" >/dev/null || return

}

# tests should not share the same floxmeta repo
floxmeta_setup() {
  export FLOX_DATA_DIR="$BATS_TEST_TMPDIR/floxdata"
}

project_teardown() {
  popd >/dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
}

setup() {
  common_test_setup
  project_setup
  home_setup test;
  floxhub_setup "$OWNER";
}

teardown() {
  project_teardown
  common_test_teardown
}

# ---------------------------------------------------------------------------- #

function make_empty_remote_env() {
  # init path environment and push to remote
  "$FLOX_BIN" init
  "$FLOX_BIN" push --owner "$OWNER"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=install,managed
@test "m1: install a package to a managed environment" {
  make_empty_remote_env

  run --separate-stderr "$FLOX_BIN" list
  assert_success
  assert_output  ""

  run "$FLOX_BIN" install hello
  assert_success
  assert_output --partial "environment $OWNER/test" # managed env output

  run --separate-stderr "$FLOX_BIN" list
  assert_success
  assert_output "hello"
}


# bats test_tags=uninstall,managed
@test "m2: uninstall a package from a managed environment" {
  make_empty_remote_env
  "$FLOX_BIN" install hello

  run "$FLOX_BIN" uninstall hello
  assert_success

  run --separate-stderr "$FLOX_BIN" list
  assert_success
  assert_output ""
}

# bats test_tags=edit,managed
@test "m3: edit a package from a managed environment" {
  make_empty_remote_env

  TMP_MANIFEST_PATH="$BATS_TEST_TMPDIR/manifest.toml"

  cat << "EOF" >> "$TMP_MANIFEST_PATH"
[install]
hello = {}
EOF

  run "$FLOX_BIN" edit -f "$TMP_MANIFEST_PATH"
  assert_success
  assert_output --partial "✅ environment successfully edited"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=managed,pull,managed:pull
@test "m4: pushed environment can be pulled" {



  mkdir a a_data;
  mkdir b b_data;

  # on machine a, create and push the environment
  export FLOX_DATA_DIR="$(pwd)/a_data"
  pushd a >/dev/null || return
  "$FLOX_BIN" init
  "$FLOX_BIN" install hello
  "$FLOX_BIN" push --owner "$OWNER"
  popd >/dev/null || return


  # on another b machine, pull the environment
  export FLOX_DATA_DIR="$(pwd)/b_data"
  pushd b >/dev/null || return
  "$FLOX_BIN" pull --remote "$OWNER/a"
  run --separate-stderr "$FLOX_BIN" list

  # assert that the environment contains the installed package
  assert_output "hello"
  popd >/dev/null || return
}



# bats test_tags=managed,update,managed:update
@test "m5: updated environment can be pulled" {
  mkdir a a_data;
  mkdir b b_data;

  # on machine a, create and push the (empty) environment
  export FLOX_DATA_DIR="$(pwd)/a_data"
  pushd a >/dev/null || return
  "$FLOX_BIN" init
  "$FLOX_BIN" push --owner "$OWNER"
  popd >/dev/null || return


  # on another b machine,
  #  - pull the environment
  #  - install a package
  #  - push the environment
  export FLOX_DATA_DIR="$(pwd)/b_data"
  pushd b >/dev/null || return
  "$FLOX_BIN" pull --remote "$OWNER/a"
  "$FLOX_BIN" install hello
  "$FLOX_BIN" push --owner "$OWNER"
  popd >/dev/null || return

  # on machine a, pull the environment
  # and check that the package is installed
  export FLOX_DATA_DIR="$(pwd)/a_data"
  pushd a >/dev/null || return
  # assert that pulling succeeds
  run "$FLOX_BIN" pull
  assert_success

  # assert that the environment contains the installed package
  run --separate-stderr "$FLOX_BIN" list
  assert_output "hello"
  popd >/dev/null || return
}


# bats test_tags=managed,diverged,managed:diverged
@test "m7: remote can not be pulled into diverged environment" {
  mkdir a a_data;
  mkdir b b_data;

  # on machine a, create and push the (empty) environment
  export FLOX_DATA_DIR="$(pwd)/a_data"
  pushd a >/dev/null || return
  "$FLOX_BIN" init
  "$FLOX_BIN" push --owner "$OWNER"
  popd >/dev/null || return


  # on another b machine,
  #  - pull the environment
  #  - install a package
  #  - push the environment
  export FLOX_DATA_DIR="$(pwd)/b_data"
  pushd b >/dev/null || return
  "$FLOX_BIN" pull --remote "$OWNER/a"
  "$FLOX_BIN" install vim
  "$FLOX_BIN" push --owner "$OWNER"
  popd >/dev/null || return

  # on machine a, pull the environment
  # and check that the package is installed
  export FLOX_DATA_DIR="$(pwd)/a_data"
  pushd a >/dev/null || return
  run "$FLOX_BIN" install emacs
  # assert that pulling succeeds
  run "$FLOX_BIN" pull
  assert_failure

  # assert that the environment contains the installed package
  assert_output --partial "diverged"
  popd >/dev/null || return
}

# ---------------------------------------------------------------------------- #

# Make sure we haven't broken regular search
# bats test_tags=managed,search,managed:search
@test "m8: search works in managed environment" {
  make_empty_remote_env

  run "$FLOX_BIN" search hello;
  assert_success;
}
