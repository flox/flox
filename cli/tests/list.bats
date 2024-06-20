#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test of rust impl of 'flox list'
#
# Tests are tentative, missing spec!
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# ---------------------------------------------------------------------------- #

# Helpers for project based tests.

project_setup() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/test"
  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" >/dev/null || return

  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.json"
}

project_teardown() {
  popd >/dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
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

# bats test_tags=list,list:catalog
@test "'flox list' lists packages of environment in the current dir; One package from nixpkgs" {
  "$FLOX_BIN" init
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json" \
    "$FLOX_BIN" install hello

  run "$FLOX_BIN" list
  assert_success
  assert_output --regexp 'hello: hello \([0-9]+\.[0-9]+(\.[0-9]+)?\)'
}

# bats test_tags=list,list:catalog,list:config
@test "'flox list --config' shows manifest content" {
  "$FLOX_BIN" init
  MANIFEST_CONTENT="$(
    cat <<-EOF
    version = 1

    [install]

    [hook]
    on-activate = "something suspicious"
EOF
  )"

  echo "$MANIFEST_CONTENT" | "$FLOX_BIN" edit -f -

  run "$FLOX_BIN" list --config
  assert_success
  assert_output "$MANIFEST_CONTENT"
}

# ---------------------------------------------------------------------------- #
