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
}
teardown() {
  project_teardown
  common_test_teardown
}

@test "upgrade hello" {
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
  "$FLOX_BIN" init
  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_OLD?}" \
    "$FLOX_BIN" install curl hello

  run "$FLOX_BIN" upgrade
  assert_success
  assert_output --partial "No packages need to be upgraded"
}

@test "check confirmation when package is up to date" {
  "$FLOX_BIN" init
  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_OLD?}" \
    "$FLOX_BIN" install curl hello

  run "$FLOX_BIN" upgrade
  assert_success
  assert_output --partial "No packages need to be upgraded"
}


# Catalog functionality tests
# ---------------------------------------------------------------------------- #

# bats test_tags=upgrade:catalog
@test "'flox upgrade' upgrades with catalog" {
  export FLOX_FEATURES_USE_CATALOG=true

  "$FLOX_BIN" init
  # create a catalog manifest
  echo "version = 1" > ".flox/env/manifest.toml"
  echo 'options.systems = ["aarch64-darwin", "x86_64-darwin", "aarch64-linux", "x86_64-linux"]' >> ".flox/env/manifest.toml"

  _FLOX_USE_CATALOG_MOCK="$TESTS_DIR/catalog_responses/hello_resolution_old.json" \
  "$FLOX_BIN" install -i hello_install_id hello

  run "$FLOX_BIN" list
  assert_success
  assert_line "hello_install_id: hello (old_version)"

  _FLOX_USE_CATALOG_MOCK="$TESTS_DIR/catalog_responses/hello_resolution.json" \
  run "$FLOX_BIN" upgrade hello_install_id
  assert_success

  run "$FLOX_BIN" list
  assert_success
  assert_line "hello_install_id: hello (version)"
}
