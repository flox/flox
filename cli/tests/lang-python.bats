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
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.yaml"
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
# catalog tests

# bats test_tags=catalog
@test "install requests with pip" {
  "$FLOX_BIN" init

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/python3_pip.yaml" \
      run "$FLOX_BIN" install -i pip python310Packages.pip python3

  assert_success
  assert_output --partial "✅ 'python3', 'pip' installed to environment"

  "$FLOX_BIN" activate -- bash "$INPUT_DATA/init/python/requests-with-pip.sh"
}

# bats test_tags=python:activate:poetry,catalog
@test "flox activate works with poetry" {
  cp -r "$INPUT_DATA"/init/python/common/* "$PROJECT_DIR/"
  cp -r "$INPUT_DATA"/init/python/poetry/* "$PROJECT_DIR/"
  # Files copied from the store are read-only
  chmod -R +w .

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/init/python_poetry.yaml" \
    run "$FLOX_BIN" init --auto-setup
  assert_success
  assert_output --partial "'poetry' installed"

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/init/python_poetry_zlib.yaml" \
    "$FLOX_BIN" install zlib

  run "$FLOX_BIN" activate -- python -m project
  assert_success
  assert_line "<class 'numpy.ndarray'>"
}

# bats test_tags=python:activate:pyproject:pip,catalog
@test "flox activate works with pyproject and pip" {
  cp -r "$INPUT_DATA"/init/python/common/* "$PROJECT_DIR/"
  cp -r "$INPUT_DATA"/init/python/pyproject-pip/* "$PROJECT_DIR/"
  # Files copied from the store are read-only
  chmod -R +w .

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/init/python_pyproject_pip.yaml" \
    run "$FLOX_BIN" init --auto-setup
  assert_success

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/init/python_pyproject_pip_zlib.yaml" \
    "$FLOX_BIN" install zlib

  run "$FLOX_BIN" activate -- python -m project
  assert_success
  assert_line "<class 'numpy.ndarray'>"
}

# bats test_tags=python:activate:requirements,catalog
@test "flox activate works with requirements.txt and pip" {
  cp -r "$INPUT_DATA"/init/python/common/* "$PROJECT_DIR/"
  cp -r "$INPUT_DATA"/init/python/requirements/* "$PROJECT_DIR/"
  # Files copied from the store are read-only
  chmod -R +w .

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/init/python_requirements.yaml" \
    run "$FLOX_BIN" init --auto-setup
  assert_success

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/init/python_requirements_zlib.yaml" \
    "$FLOX_BIN" install zlib

  run "$FLOX_BIN" activate -- python -m project
  assert_success
  assert_line "<class 'numpy.ndarray'>"
}

# bats test_tags=init:python:auto-setup,init:python:auto-setup:bash,catalog
@test "verify auto-setup Python venv activation: bash" {
  OWNER="owner"
  NAME="name"
  echo "requests" > requirements.txt
  [ ! -e .flox ] || "$FLOX_BIN" delete -f
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/init/python_requests.yaml" \
    "$FLOX_BIN" init --auto-setup --name "$NAME"
  FLOX_SHELL="bash" run "$FLOX_BIN" activate -- type deactivate
  assert_success
  assert_line --partial "deactivate is a function"
}

# bats test_tags=init:python:auto-setup,init:python:auto-setup:zsh,catalog
@test "verify auto-setup Python venv activation: zsh" {
  OWNER="owner"
  NAME="name"
  echo "requests" > requirements.txt
  [ ! -e .flox ] || "$FLOX_BIN" delete -f
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/init/python_requests.yaml" \
    "$FLOX_BIN" init --auto-setup --name "$NAME"
  FLOX_SHELL="zsh" run "$FLOX_BIN" activate -- type deactivate
  assert_success
  assert_line --partial "deactivate is a shell function"
}

# bats test_tags=init:python:auto-setup,init:python:auto-setup:fish,catalog
@test "verify auto-setup Python venv activation: fish" {
  OWNER="owner"
  NAME="name"
  echo "requests" > requirements.txt
  [ ! -e .flox ] || "$FLOX_BIN" delete -f
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/init/python_requests.yaml" \
    "$FLOX_BIN" init --auto-setup --name "$NAME"
  FLOX_SHELL="fish" run "$FLOX_BIN" activate -- type deactivate
  assert_success
  assert_line --partial "deactivate is a function with definition"
}

# bats test_tags=init:python:auto-setup,init:python:auto-setup:tcsh,catalog
@test "verify auto-setup Python venv activation: tcsh" {
  OWNER="owner"
  NAME="name"
  echo "requests" > requirements.txt
  [ ! -e .flox ] || "$FLOX_BIN" delete -f
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/init/python_requests.yaml" \
    "$FLOX_BIN" init --auto-setup --name "$NAME"
  FLOX_SHELL="tcsh" run "$FLOX_BIN" activate -- which deactivate
  assert_success
  assert_line --partial "aliased to test \$?_OLD_VIRTUAL_PATH != 0 && setenv PATH "
  # ... and a bunch of other stuff ending with:
  assert_line --partial " && unalias deactivate"
}
