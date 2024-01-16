#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test if LD_AUDIT and ld-floxlib.so works with flox.
#
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# bats file_tags=end2end,ld-floxlib

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
  cp ./harnesses/ld-floxlib/* "$PROJECT_DIR"
  pushd "$PROJECT_DIR" > /dev/null || return
  "$FLOX_BIN" init
  sed -i \
    's/from = { type = "github", owner = "NixOS", repo = "nixpkgs" }/from = { type = "github", owner = "NixOS", repo = "nixpkgs", rev = "e8039594435c68eb4f780f3e9bf3972a7399c4b1" }/' \
    "$PROJECT_DIR/.flox/env/manifest.toml"
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
#
@test "test ld-floxlib.so on Linux only" {
  if [ $(uname -s) != "Linux" ]; then
    skip "not Linux"
  fi

  run "$FLOX_BIN" install gcc glibc giflib
  assert_success
  assert_output --partial "✅ 'gcc' installed to environment"
  assert_output --partial "✅ 'glibc' installed to environment"
  assert_output --partial "✅ 'giflib' installed to environment"

  #SHELL=bash run expect -d "$TESTS_DIR/ld-floxlib.exp" "$PROJECT_DIR"
  #assert_success

  ### Verify environment
  run "$FLOX_BIN" activate -- sh ./verify-environment.sh
  assert_success

  ### Test 1: load libraries found in $FLOX_ENV_LIB_DIRS last
  run "$FLOX_BIN" activate -- sh ./test-load-library-last.sh
  assert_success

  ### Test 2: confirm LD_AUDIT can find missing libraries
  run "$FLOX_BIN" activate -- sh -c "cc -o print-gif-info ./print-gif-info.c -lgif && ./print-gif-info ./flox-edge.gif"
  assert_output --partial "GIF Information for: ./flox-edge.gif
  assert_output --partial "Number of frames: 0
  assert_output --partial "Width: 270 pixels
  assert_output --partial "Height: 137 pixels
}
