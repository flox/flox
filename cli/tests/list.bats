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

  export FLOX_FEATURES_USE_CATALOG=true
  export _FLOX_USE_CATALOG_MOCK="$TESTS_DIR/catalog_responses/empty_responses.json"
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

@test "'flox list' lists packages of environment in the current dir; fails if no env found" {
  run "$FLOX_BIN" list
  assert_failure
}

@test "'flox list' lists packages of environment in the current dir; No package" {
  export FLOX_FEATURES_USE_CATALOG=false

  "$FLOX_BIN" init
  run "$FLOX_BIN" list
  assert_success
}

@test "'flox list' lists packages of environment in the current dir; One package from nixpkgs" {
  export FLOX_FEATURES_USE_CATALOG=false

  "$FLOX_BIN" init
  "$FLOX_BIN" install hello

  run "$FLOX_BIN" list
  assert_success
  assert_output --regexp 'hello: hello \([0-9]+\.[0-9]+(\.[0-9]+)?\)'
}

@test "'flox list' lists packages of environment in the current dir; shows different paths" {
  export FLOX_FEATURES_USE_CATALOG=false

  "$FLOX_BIN" init
  "$FLOX_BIN" install python310Packages.pip

  run "$FLOX_BIN" list
  assert_success
  assert_output --regexp - <<EOF
pip: python310Packages.pip \([0-9]+\.[0-9]+(\.[0-9]+)?\)
EOF
}

@test "'flox list' lists packages of environment in the current dir; shows different id" {
  export FLOX_FEATURES_USE_CATALOG=false

  "$FLOX_BIN" init
  "$FLOX_BIN" install --id greeting hello

  run "$FLOX_BIN" list
  assert_success
  assert_output --regexp - <<EOF
greeting: hello \([0-9]+\.[0-9]+(\.[0-9]+)?\)
EOF
}

# bats test_tags=list,list:config
@test "'flox list --config' shows manifest content" {
  export FLOX_FEATURES_USE_CATALOG=false

  "$FLOX_BIN" init
  MANIFEST_CONTENT="$(
    cat <<-EOF
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

# bats test_tags=list,list:not-applicable
@test "'flox list' hides packages not installed for the current system" {
  export FLOX_FEATURES_USE_CATALOG=false

  "$FLOX_BIN" init
  MANIFEST_CONTENT="$(
    cat <<-EOF
    [options]
    systems = [ "$NIX_SYSTEM" ]
    [install]
    hello.pkg-path = "hello"
    htop = { pkg-path = "htop", systems = [] }
EOF

  )"

  echo "$MANIFEST_CONTENT" | "$FLOX_BIN" edit -f -

  run "$FLOX_BIN" list -n
  assert_success
  assert_output "hello"
}

# ---------------------------------------------------------------------------- #

# https://github.com/flox/flox/issues/1039
# bats test_tags=list,list:tolerates-missing-version
@test "'flox list' tolerates missing version" {
  export FLOX_FEATURES_USE_CATALOG=false

  "$FLOX_BIN" init
  # `influxdb does not have a version attribute set in nixpkgs (2024-02-19)
  # todo: replace with a more predicatable/smaller example
  "$FLOX_BIN" install influxdb2
  run "$FLOX_BIN" list
  assert_success
  assert_output "influxdb2: influxdb2 (N/A)"
}

# ------------------------------ Catalog Tests ------------------------------- #
# ---------------------------------------------------------------------------- #

# bats test_tags=list,list:catalog
@test "catalog: 'flox list' lists packages of environment in the current dir; One package from nixpkgs" {
  "$FLOX_BIN" init
  _FLOX_USE_CATALOG_MOCK="$TESTS_DIR/catalog_responses/resolve/hello.json" \
    "$FLOX_BIN" install hello

  run "$FLOX_BIN" list
  assert_success
  assert_output --regexp 'hello: hello \([0-9]+\.[0-9]+(\.[0-9]+)?\)'
}

# ---------------------------------------------------------------------------- #
