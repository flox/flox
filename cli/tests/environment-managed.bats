#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test the managed environment feature of flox.
# * Tests whether flox commands work as expected in a managed environment
# * Tests conversion of a local environments to managed environments
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# ---------------------------------------------------------------------------- #

# Helpers for project based tests.

project_setup() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/project-managed-${BATS_TEST_NUMBER?}"
  export PROJECT_NAME="${PROJECT_DIR##*/}"
  export OWNER="owner"

  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" > /dev/null || return

}

# tests should not share the same floxmeta repo
floxmeta_setup() {
  export FLOX_DATA_DIR="$BATS_TEST_TMPDIR/floxdata"
}

project_teardown() {
  popd > /dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
}

setup() {
  common_test_setup
  setup_isolated_flox
  project_setup
  floxhub_setup "$OWNER"
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

dot_flox_exists() {
  # Since the return value is based on the exit code of `test`:
  # 0 = true
  # 1 = false
  [[ -d "$PROJECT_DIR/.flox" ]]
}

# ---------------------------------------------------------------------------- #

# bats test_tags=install,managed
@test "m1: install a package to a managed environment" {
  make_empty_remote_env

  run --separate-stderr "$FLOX_BIN" list --name
  assert_success
  assert_output ""

  run "$FLOX_BIN" install hello
  assert_success
  assert_output --partial "environment '$OWNER/project-managed-${BATS_TEST_NUMBER}'" # managed env output

  run --separate-stderr "$FLOX_BIN" list --name
  assert_success
  assert_output "hello"
}

# bats test_tags=uninstall,managed
@test "m2: uninstall a package from a managed environment" {
  make_empty_remote_env
  "$FLOX_BIN" install hello

  run "$FLOX_BIN" uninstall hello
  assert_success

  run --separate-stderr "$FLOX_BIN" list --name
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
  assert_output --partial "✅ Environment successfully updated."
}

# ---------------------------------------------------------------------------- #

# bats test_tags=managed,pull,managed:pull
@test "m4: pushed environment can be pulled" {

  mkdir a a_data
  mkdir b b_data

  # on machine a, create and push the environment
  export FLOX_DATA_DIR="$(pwd)/a_data"
  pushd a > /dev/null || return
  "$FLOX_BIN" init
  "$FLOX_BIN" install hello
  "$FLOX_BIN" push --owner "$OWNER"
  popd > /dev/null || return

  # on another b machine, pull the environment
  export FLOX_DATA_DIR="$(pwd)/b_data"
  pushd b > /dev/null || return
  "$FLOX_BIN" pull --remote "$OWNER/a"
  run --separate-stderr "$FLOX_BIN" list --name

  # assert that the environment contains the installed package
  assert_output "hello"
  popd > /dev/null || return
}

# bats test_tags=managed,update,managed:update
@test "m5: updated environment can be pulled" {
  mkdir a a_data
  mkdir b b_data

  # on machine a, create and push the (empty) environment
  export FLOX_DATA_DIR="$(pwd)/a_data"
  pushd a > /dev/null || return
  "$FLOX_BIN" init
  "$FLOX_BIN" push --owner "$OWNER"
  popd > /dev/null || return

  # on another b machine,
  #  - pull the environment
  #  - install a package
  #  - push the environment
  export FLOX_DATA_DIR="$(pwd)/b_data"
  pushd b > /dev/null || return
  "$FLOX_BIN" pull --remote "$OWNER/a"
  "$FLOX_BIN" install hello
  "$FLOX_BIN" push --owner "$OWNER"
  popd > /dev/null || return

  # on machine a, pull the environment
  # and check that the package is installed
  export FLOX_DATA_DIR="$(pwd)/a_data"
  pushd a > /dev/null || return
  # assert that pulling succeeds
  run "$FLOX_BIN" pull
  assert_success

  # assert that the environment contains the installed package
  run --separate-stderr "$FLOX_BIN" list --name
  assert_output "hello"
  popd > /dev/null || return
}

# bats test_tags=managed,diverged,managed:diverged
@test "m7: remote can not be pulled into diverged environment" {
  mkdir a a_data
  mkdir b b_data

  # on machine a, create and push the (empty) environment
  export FLOX_DATA_DIR="$(pwd)/a_data"
  pushd a > /dev/null || return
  "$FLOX_BIN" init
  "$FLOX_BIN" push --owner "$OWNER"
  popd > /dev/null || return

  # on another b machine,
  #  - pull the environment
  #  - install a package
  #  - push the environment
  export FLOX_DATA_DIR="$(pwd)/b_data"
  pushd b > /dev/null || return
  "$FLOX_BIN" pull --remote "$OWNER/a"
  "$FLOX_BIN" install vim
  "$FLOX_BIN" push --owner "$OWNER"
  popd > /dev/null || return

  # on machine a, pull the environment
  # and check that the package is installed
  export FLOX_DATA_DIR="$(pwd)/a_data"
  pushd a > /dev/null || return
  run "$FLOX_BIN" install emacs
  # assert that pulling fails
  run "$FLOX_BIN" pull
  assert_failure
  # assert that the environment contains the installed package
  assert_output --partial "diverged"

  # assert that pulling with `--force` succeeds
  run "$FLOX_BIN" pull --force
  assert_success

  popd > /dev/null || return
}

# bats test_tags=managed,diverged,managed:diverged-upstream
@test "m8: remote can be force pulled into diverged environment" {
  mkdir a
  mkdir b

  # on machine a, create and push the (empty) environment
  pushd a > /dev/null || return
  "$FLOX_BIN" init
  FLOX_DATA_DIR="$(pwd)/a_data" "$FLOX_BIN" push --owner "$OWNER"
  popd > /dev/null || return

  pushd b > /dev/null || return
  FLOX_DATA_DIR="$(pwd)/b_data" "$FLOX_BIN" pull --remote "$OWNER/a"
  FLOX_DATA_DIR="$(pwd)/b_data" "$FLOX_BIN" install vim
  popd > /dev/null || return

  pushd a > /dev/null || return
  FLOX_DATA_DIR="$(pwd)/a_data" "$FLOX_BIN" install emacs
  FLOX_DATA_DIR="$(pwd)/a_data" "$FLOX_BIN" push
  popd > /dev/null || return

  pushd b > /dev/null || return
  FLOX_DATA_DIR="$(pwd)/b_data" "$FLOX_BIN" push --force
  popd > /dev/null || return

  pushd a > /dev/null || return
  FLOX_DATA_DIR="$(pwd)/a_data" run "$FLOX_BIN" pull
  assert_failure
  FLOX_DATA_DIR="$(pwd)/a_data" run "$FLOX_BIN" pull --force
  assert_success
  popd > /dev/null || return
}

# ---------------------------------------------------------------------------- #

# Make sure we haven't broken regular search
# bats test_tags=managed,search,managed:search
@test "m8: search works in managed environment" {
  make_empty_remote_env

  run "$FLOX_BIN" search hello
  assert_success
}

# ---------------------------------------------------------------------------- #

# Make sure we haven't activate
# bats test_tags=managed,activate,managed:activate
@test "m9: activate works in managed environment" {
  make_empty_remote_env
  "$FLOX_BIN" install hello

  # TODO: flox will set HOME if it doesn't match the home of the user with
  # current euid. I'm not sure if we should change that, but for now just set
  # USER to REAL_USER.
  FLOX_SHELL=bash USER="$REAL_USER" run -0 expect "$TESTS_DIR/activate/hello.exp" "$PROJECT_DIR"
  assert_output --regexp "$FLOX_CACHE_DIR/run/owner/.+\..+\..+/bin/hello"
  refute_output "not found"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=managed,delete,managed:delete
@test "m10: deletes existing environment" {
  # This test asserts before and after state of the home directory.
  # Remaining state from other tests may cause this test misbehave.
  # Hence, use a clean home directory, for this test rather than the shared one.
  home_setup test

  make_empty_remote_env

  run dot_flox_exists
  assert_success

  run "$FLOX_BIN" delete
  assert_success

  run dot_flox_exists
  assert_failure

  run ls -lA "$FLOX_DATA_DIR/links"
  assert_output "total 0"
}

# test that non-pushed environments can be deleted
# and are recreated at the current pushed state.
# bats test_tags=managed,delete,managed:fresh-deleted
@test "m11: uses fresh branch after delete" {
  make_empty_remote_env
  "$FLOX_BIN" install vim

  run "$FLOX_BIN" delete
  assert_success

  run dot_flox_exists
  assert_failure

  # when recreating an environment, a new branch should be used
  run "$FLOX_BIN" pull --remote "$OWNER/project-managed-${BATS_TEST_NUMBER}"
  assert_success

  "$FLOX_BIN" install emacs
  run "$FLOX_BIN" list --name
  assert_output --partial "emacs"
  refute_output "vim"
}

@test "sanity check upgrade works for managed environments" {
  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_OLD?}" \
    make_empty_remote_env

  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_OLD?}" \
    "$FLOX_BIN" install hello

  # After an update, nixpkgs is the new nixpkgs, but hello is still from the
  # old one.
  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_NEW?}" \
    "$FLOX_BIN" update

  run "$FLOX_BIN" upgrade
  assert_output --partial "Upgraded 'hello'"
}

# ---------------------------------------------------------------------------- #
