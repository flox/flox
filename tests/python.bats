#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test if python stuff works with flox.
#
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# bats file_tags=end2end,python

# ---------------------------------------------------------------------------- #

setup_file() {
  common_file_setup
}

# ---------------------------------------------------------------------------- #

# Helpers for project based tests.

project_setup() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/project-${BATS_TEST_NUMBER?}"
  export PROJECT_NAME="${PROJECT_DIR##*/}"
  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" >/dev/null || return
}

project_teardown() {
  popd >/dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
  unset PROJECT_NAME
}

# ---------------------------------------------------------------------------- #

setup() {
  common_test_setup
  project_setup
}
teardown() {
  project_teardown
  common_test_teardown
}

# ---------------------------------------------------------------------------- #
#
@test "install requests with pip" {
  "$FLOX_BIN" init
  sed -i \
    's/from = { type = "github", owner = "NixOS", repo = "nixpkgs" }/from = { type = "github", owner = "NixOS", repo = "nixpkgs", rev = "e8039594435c68eb4f780f3e9bf3972a7399c4b1" }/' \
    "$PROJECT_DIR/.flox/env/manifest.toml"

  run "$FLOX_BIN" install -i pip python310Packages.pip python3

  assert_success
  assert_output --partial "✅ 'pip' installed to environment"
  assert_output --partial "✅ 'python3' installed to environment"

  FLOX_SHELL=bash "$FLOX_BIN" activate -- bash "$TESTS_DIR/python/requests-with-pip.sh"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=python:activate:poetry
@test "flox activate works with poetry" {
  cp -r "$TESTS_DIR"/python/single-dependency/common/* "$PROJECT_DIR/"
  cp -r "$TESTS_DIR"/python/single-dependency/poetry/* "$PROJECT_DIR/"

  run "$FLOX_BIN" init --auto-setup
  assert_success
  assert_output --partial "'poetry' installed"

  "$FLOX_BIN" install zlib
  run "$FLOX_BIN" activate -- python -m project
  assert_success
  assert_line "<class 'numpy.ndarray'>"
}

# bats test_tags=python:activate:pyproject:pip
@test "flox activate works with pyproject and pip" {
  cp -r "$TESTS_DIR"/python/single-dependency/common/* "$PROJECT_DIR/"
  cp -r "$TESTS_DIR"/python/single-dependency/pyproject-pip/* "$PROJECT_DIR/"

  run "$FLOX_BIN" init --auto-setup
  assert_success

  "$FLOX_BIN" install zlib
  run "$FLOX_BIN" activate -- python -m project
  assert_success
  assert_line "<class 'numpy.ndarray'>"
}

# bats test_tags=python:activate:requirements
@test "flox activate works with requirements.txt and pip" {
  cp -r "$TESTS_DIR"/python/single-dependency/common/* "$PROJECT_DIR/"
  cp -r "$TESTS_DIR"/python/single-dependency/requirements/* "$PROJECT_DIR/"

  run "$FLOX_BIN" init --auto-setup
  assert_success

  "$FLOX_BIN" install zlib
  run "$FLOX_BIN" activate -- python -m project
  assert_success
  assert_line "<class 'numpy.ndarray'>"
}

# bats test_tags=init:python:auto-setup,init:python:auto-setup:bash
@test "verify auto-setup Python venv activation: bash" {
  OWNER="owner"
  NAME="name"
  echo "requests" > requirements.txt
  [ ! -e .flox ] || "$FLOX_BIN" delete -f
  "$FLOX_BIN" init --auto-setup --name "$NAME"
  FLOX_SHELL="bash" run "$FLOX_BIN" activate -- type deactivate
  assert_success
  assert_line --partial "deactivate is a function"
}

# bats test_tags=init:python:auto-setup,init:python:auto-setup:fish
@test "verify auto-setup Python venv activation: fish" {
  OWNER="owner"
  NAME="name"
  echo "requests" > requirements.txt
  [ ! -e .flox ] || "$FLOX_BIN" delete -f
  "$FLOX_BIN" init --auto-setup --name "$NAME"
  FLOX_SHELL="fish" run "$FLOX_BIN" activate -- type deactivate
  assert_success
  assert_line --partial "deactivate is a function with definition"
}

# bats test_tags=init:python:auto-setup,init:python:auto-setup:tcsh
@test "verify auto-setup Python venv activation: tcsh" {
  OWNER="owner"
  NAME="name"
  echo "requests" > requirements.txt
  [ ! -e .flox ] || "$FLOX_BIN" delete -f
  "$FLOX_BIN" init --auto-setup --name "$NAME"
  FLOX_SHELL="tcsh" run "$FLOX_BIN" activate -- which deactivate
  assert_success
  assert_line --partial "aliased to test \$?_OLD_VIRTUAL_PATH != 0 && setenv PATH "
  # ... and a bunch of other stuff ending with:
  assert_line --partial " && unalias deactivate"
}

# bats test_tags=init:python:auto-setup,init:python:auto-setup:zsh
@test "verify auto-setup Python venv activation: zsh" {
  OWNER="owner"
  NAME="name"
  echo "requests" > requirements.txt
  [ ! -e .flox ] || "$FLOX_BIN" delete -f
  "$FLOX_BIN" init --auto-setup --name "$NAME"
  FLOX_SHELL="zsh" run "$FLOX_BIN" activate -- type deactivate
  assert_success
  assert_line --partial "deactivate is a shell function"
}
