#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test `flox update`
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# ---------------------------------------------------------------------------- #

# Helpers for project based tests.

project_setup() {
  export PROJECT_NAME="test"
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/$PROJECT_NAME"
  export MANIFEST_PATH="$PROJECT_DIR/.flox/env/manifest.toml"
  export LOCK_PATH="$PROJECT_DIR/.flox/env/manifest.lock"
  export TMP_MANIFEST_PATH="${BATS_TEST_TMPDIR}/manifest.toml"
  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" > /dev/null || return
}

project_teardown() {
  popd > /dev/null || return
  rm -rf "${PROJECT_DIR?}"
  rm -f "${TMP_MANIFEST_PATH?}"
  unset PROJECT_DIR
  unset MANIFEST_PATH
  unset TMP_MANIFEST_PATH
}

assert_old_hello() {
  run jq -r ".packages.\"$NIX_SYSTEM\".hello.input.attrs.narHash" "$LOCK_PATH"
  assert_success
  assert_output "$PKGDB_NIXPKGS_NAR_HASH_OLD"
}

assert_new_hello() {
  run jq -r ".packages.\"$NIX_SYSTEM\".hello.input.attrs.narHash" "$LOCK_PATH"
  assert_success
  assert_output "$PKGDB_NIXPKGS_NAR_HASH_NEW"
}

# ---------------------------------------------------------------------------- #

setup() {
  common_test_setup
  setup_isolated_flox
  project_setup
  export FLOX_FEATURES_USE_CATALOG=true
  export _FLOX_USE_CATALOG_MOCK="$TEST_DATA_DIR/empty.json"
}
teardown() {
  project_teardown
  common_test_teardown
}


# ---------------------------------------------------------------------------- #
# pkgdb tests

@test "upgrade hello" {
  export FLOX_FEATURES_USE_CATALOG=false
  rm -f "$GLOBAL_MANIFEST_LOCK"

  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_OLD?}" \
    "$FLOX_BIN" init
  "$FLOX_BIN" install hello

  # nixpkgs and hello are both locked to the old nixpkgs
  run jq -r '.registry.inputs.nixpkgs.from.narHash' "$LOCK_PATH"
  assert_success
  assert_output "$PKGDB_NIXPKGS_NAR_HASH_OLD"

  assert_old_hello

  # After an update, nixpkgs is the new nixpkgs, but hello is still from the
  # old one.
  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_NEW?}" \
    "$FLOX_BIN" update
  run jq -r '.registry.inputs.nixpkgs.from.narHash' "$LOCK_PATH"
  assert_success
  assert_output "$PKGDB_NIXPKGS_NAR_HASH_NEW"

  assert_old_hello

  run "$FLOX_BIN" upgrade
  assert_output --partial "Upgraded 'hello'"
  assert_new_hello
}

@test "upgrade by group" {
  export FLOX_FEATURES_USE_CATALOG=false
  rm -f "$GLOBAL_MANIFEST_LOCK"

  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_OLD?}" \
   "$FLOX_BIN" init
  cat << "EOF" > "$TMP_MANIFEST_PATH"
[install]
hello = { pkg-group = "blue" }
EOF

  "$FLOX_BIN" edit -f "$TMP_MANIFEST_PATH"

  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_NEW?}" \
    "$FLOX_BIN" update
  assert_old_hello

  run "$FLOX_BIN" upgrade blue
  assert_output --partial "Upgraded 'hello'"
  assert_new_hello
}

@test "upgrade toplevel group" {
  export FLOX_FEATURES_USE_CATALOG=false
  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_OLD?}" \
    "$FLOX_BIN" init
  "$FLOX_BIN" install hello
  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_NEW?}" \
    "$FLOX_BIN" update
  assert_old_hello

  run "$FLOX_BIN" upgrade toplevel
  assert_output --partial "Upgraded 'hello'"
  assert_new_hello
}

@test "upgrade by iid" {
  export FLOX_FEATURES_USE_CATALOG=false
  rm -f "$GLOBAL_MANIFEST_LOCK"

  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_OLD?}" \
    "$FLOX_BIN" init
  "$FLOX_BIN" install hello
  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_NEW?}" \
    "$FLOX_BIN" update
  assert_old_hello

  run "$FLOX_BIN" upgrade hello
  assert_output --partial "Upgraded 'hello'"
  assert_new_hello
}

@test "upgrade errors on iid in group with other packages" {
  export FLOX_FEATURES_USE_CATALOG=false
  "$FLOX_BIN" init
  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_OLD?}" \
    "$FLOX_BIN" install curl hello

  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_NEW?}" \
    "$FLOX_BIN" update

  run "$FLOX_BIN" upgrade hello
  assert_failure
  assert_output --partial "package in the group 'toplevel' with multiple packages"
}

@test "check confirmation when all packages are up to date" {
  export FLOX_FEATURES_USE_CATALOG=false
  "$FLOX_BIN" init
  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_OLD?}" \
    "$FLOX_BIN" install curl hello

  run "$FLOX_BIN" upgrade
  assert_success
  assert_output --partial "No packages need to be upgraded"
}

# ---------------------------------------------------------------------------- #
# catalog tests

@test "catalog: upgrade hello" {
  skip "FIXME: upgrades package but reports that no packages were upgraded"

  "$FLOX_BIN" init
  _FLOX_USE_CATALOG_MOCK="$TEST_DATA_DIR/resolve/old_hello.json" "$FLOX_BIN" install hello

  old_hello_response_rev=$(jq -r '.[0].[0].pages.[0].page' "$TEST_DATA_DIR/resolve/old_hello.json")
  old_hello_locked_rev=$(jq -r '.packages.[0].rev_count' "$LOCK_PATH")
  assert_equal "$old_hello_locked_rev" "$old_hello_response_rev" 

  _FLOX_USE_CATALOG_MOCK="$TEST_DATA_DIR/resolve/hello.json" \
    run "$FLOX_BIN" upgrade
  assert_output --partial "Upgraded 'hello'"
  hello_response_rev=$(jq -r '.[0].[0].pages.[0].page' "$TEST_DATA_DIR/resolve/hello.json")
  hello_locked_rev=$(jq -r '.packages.[0].rev_count' "$LOCK_PATH")
  assert_equal "$hello_locked_rev" "$hello_response_rev"
  
  assert_not_equal "$old_hello_locked_rev" "$hello_locked_rev"
}

@test "catalog: upgrade by group" {
  skip "FIXME: upgrades package but reports that no packages were upgraded"

  "$FLOX_BIN" init
  cp "$MANIFEST_PATH" "$TMP_MANIFEST_PATH"
  tomlq -i -t '.install.hello."pkg-path" = "hello"' "$TMP_MANIFEST_PATH"
  tomlq -i -t '.install.hello."pkg-group" = "blue"' "$TMP_MANIFEST_PATH"
  _FLOX_USE_CATALOG_MOCK="$TEST_DATA_DIR/resolve/old_hello.json" "$FLOX_BIN" edit -f "$TMP_MANIFEST_PATH"

  old_hello_response_rev=$(jq -r '.[0].[0].pages.[0].page' "$TEST_DATA_DIR/resolve/old_hello.json")
  old_hello_locked_rev=$(jq -r '.packages.[0].rev_count' "$LOCK_PATH")
  assert_equal "$old_hello_locked_rev" "$old_hello_response_rev"

  # add the package group

  _FLOX_USE_CATALOG_MOCK="$TEST_DATA_DIR/resolve/old_hello.json" \
    run "$FLOX_BIN" upgrade blue
  hello_response_rev=$(jq -r '.[0].[0].pages.[0].page' "$TEST_DATA_DIR/resolve/hello.json")
  hello_locked_rev=$(jq -r '.packages.[0].rev_count' "$LOCK_PATH")
  assert_equal "$hello_locked_rev" "$hello_response_rev"
  assert_output --partial "Upgraded 'hello'"
  
  assert_not_equal "$old_hello_locked_rev" "$hello_locked_rev"
}

@test "catalog: upgrade toplevel group" {
  skip "FIXME: upgrades package but reports that no packages were upgraded"

  "$FLOX_BIN" init
  _FLOX_USE_CATALOG_MOCK="$TEST_DATA_DIR/resolve/old_hello.json" "$FLOX_BIN" install hello

  old_hello_response_rev=$(jq -r '.[0].[0].pages.[0].page' "$TEST_DATA_DIR/resolve/old_hello.json")
  old_hello_locked_rev=$(jq -r '.packages.[0].rev_count' "$LOCK_PATH")
  assert_equal "$old_hello_locked_rev" "$old_hello_response_rev"

  _FLOX_USE_CATALOG_MOCK="$TEST_DATA_DIR/resolve/hello.json" \
    run "$FLOX_BIN" upgrade toplevel
  assert_output --partial "Upgraded 'hello'"
  hello_response_rev=$(jq -r '.[0].[0].pages.[0].page' "$TEST_DATA_DIR/resolve/hello.json")
  hello_locked_rev=$(jq -r '.packages.[0].rev_count' "$LOCK_PATH")
  assert_equal "$hello_locked_rev" "$hello_response_rev"
  
  assert_not_equal "$old_hello_locked_rev" "$hello_locked_rev"
}

@test "catalog: upgrade by iid" {
  skip "FIXME: upgrades package but reports that no packages were upgraded"

  "$FLOX_BIN" init
  _FLOX_USE_CATALOG_MOCK="$TEST_DATA_DIR/resolve/old_hello.json" "$FLOX_BIN" install hello

  old_hello_response_rev=$(jq -r '.[0].[0].pages.[0].page' "$TEST_DATA_DIR/resolve/old_hello.json")
  old_hello_locked_rev=$(jq -r '.packages.[0].rev_count' "$LOCK_PATH")
  assert_equal "$old_hello_locked_rev" "$old_hello_response_rev"

  _FLOX_USE_CATALOG_MOCK="$TEST_DATA_DIR/resolve/hello.json" \
    run "$FLOX_BIN" upgrade hello
  assert_output --partial "Upgraded 'hello'"
  hello_response_rev=$(jq -r '.[0].[0].pages.[0].page' "$TEST_DATA_DIR/resolve/hello.json")
  hello_locked_rev=$(jq -r '.packages.[0].rev_count' "$LOCK_PATH")
  assert_equal "$hello_locked_rev" "$hello_response_rev"
  
  assert_not_equal "$old_hello_locked_rev" "$hello_locked_rev"
}

@test "catalog: upgrade errors on iid in group with other packages" {
  skip "FIXME: upgrades package but reports that no packages were upgraded"

  "$FLOX_BIN" init
  _FLOX_USE_CATALOG_MOCK="$TEST_DATA_DIR/resolve/curl_hello.json" "$FLOX_BIN" install curl hello

  export _FLOX_USE_CATALOG_MOCK="$TEST_DATA_DIR/resolve/hello.json"
  run "$FLOX_BIN" upgrade hello
  assert_output --partial "package in the group 'toplevel' with multiple packages"
}

@test "catalog: check confirmation when all packages are up to date" {
  # This will pass right now, but that's because _all_ of these tests get
  # the "no packages need to be upgraded" message. Unskip once we get a real
  # signal from these tests
  skip "FIXME: upgrades package but reports that no packages were upgraded"
  export FLOX_FEATURES_USE_CATALOG=true

  "$FLOX_BIN" init
  _FLOX_USE_CATALOG_MOCK="$TEST_DATA_DIR/resolve/curl_hello.json" "$FLOX_BIN" install curl hello

  _FLOX_USE_CATALOG_MOCK="$TEST_DATA_DIR/resolve/curl_hello.json" \
    run "$FLOX_BIN" upgrade
  assert_success
  assert_output --partial "No packages need to be upgraded"
}
