#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test if Go works with flox.
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# bats file_tags=end2end,go

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
  pushd "$PROJECT_DIR" > /dev/null || return
  export FLOX_FEATURES_USE_CATALOG=true
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.json"
}

project_teardown() {
  popd > /dev/null || return
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

@test "'flox init' sets up a local working Go module environment" {
  export FLOX_FEATURES_USE_CATALOG=false
  cp -r "$INPUT_DATA"/init/go/common/* "$PROJECT_DIR/"
  cp -r "$INPUT_DATA"/init/go/module/* "$PROJECT_DIR/"
  # Files copied from the store are read-only
  chmod -R +w .

  run "$FLOX_BIN" init --auto-setup
  assert_success
  assert_line --partial "'go' installed"

  run "$FLOX_BIN" activate -- go version
  assert_success
  assert_line --partial "go version go1."

  run "$FLOX_BIN" activate -- go build
  assert_success
}

# ---------------------------------------------------------------------------- #

@test "'flox init' sets up a local working Go workspace environment" {
  export FLOX_FEATURES_USE_CATALOG=false
  cp -r "$INPUT_DATA"/init/go/common/* "$PROJECT_DIR/"
  cp -r "$INPUT_DATA"/init/go/module/* "$PROJECT_DIR/"
  cp -r "$INPUT_DATA"/init/go/workspace/* "$PROJECT_DIR/"
  # Files copied from the store are read-only
  chmod -R +w .

  run "$FLOX_BIN" init --auto-setup
  assert_success
  assert_line --partial "'go' installed"

  run "$FLOX_BIN" activate -- go version
  assert_success
  assert_line --partial "go version go1."

  run "$FLOX_BIN" activate -- go build
  assert_success
}

# ---------------------------------------------------------------------------- #
# catalog tests

# bats test_tags=catalog
@test "catalog: 'flox init' sets up a local working Go module environment" {
  cp -r "$INPUT_DATA"/init/go/common/* "$PROJECT_DIR/"
  cp -r "$INPUT_DATA"/init/go/module/* "$PROJECT_DIR/"
  # Files copied from the store are read-only
  chmod -R +w .

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/init/go.json" \
    run "$FLOX_BIN" init --auto-setup

  assert_success
  assert_line --partial "'go' installed"

  run "$FLOX_BIN" activate -- go version
  assert_success
  assert_line --partial "go version go1."

  run "$FLOX_BIN" activate -- go build
  assert_success
}

# bats test_tags=catalog
@test "catalog: 'flox init' sets up a local working Go workspace environment" {
  cp -r "$INPUT_DATA"/init/go/common/* "$PROJECT_DIR/"
  cp -r "$INPUT_DATA"/init/go/module/* "$PROJECT_DIR/"
  cp -r "$INPUT_DATA"/init/go/workspace/* "$PROJECT_DIR/"
  # Files copied from the store are read-only
  chmod -R +w .

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/init/go.json" \
    run "$FLOX_BIN" init --auto-setup

  assert_success
  assert_line --partial "'go' installed"

  run "$FLOX_BIN" activate -- go version
  assert_success
  assert_line --partial "go version go1."

  run "$FLOX_BIN" activate -- go build
  assert_success
}
