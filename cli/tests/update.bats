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
  export PROJECT_NAME="test";
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/$PROJECT_NAME"
  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" >/dev/null || return
}

project_teardown() {
  popd >/dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
}

OLD_NAR_HASH="sha256-1UGacsv5coICyvAzwuq89v9NsS00Lo8sz22cDHwhnn8="
NEW_NAR_HASH="sha256-5uA6jKckTf+DCbVBNKsmT5pUT/7Apt5tNdpcbLnPzFI="
GLOBAL_MANIFEST_LOCK="$FLOX_CONFIG_HOME/global-manifest.lock"

# ---------------------------------------------------------------------------- #

setup() {
  common_test_setup
  project_setup
}
teardown() {
  project_teardown
  common_test_teardown
}

@test "update bumps nixpkgs" {
  "$FLOX_BIN" init
  
  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_OLD?}" \
    run "$FLOX_BIN" update
  assert_output --partial "Locked all inputs"
  run jq -r '.registry.inputs.nixpkgs.from.narHash' .flox/env/manifest.lock
  assert_success
  assert_output "$OLD_NAR_HASH"
  
  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_NEW?}" \
    run "$FLOX_BIN" update
  assert_success
  assert_output --partial "Updated:"
  assert_output --partial "nixpkgs"
  run jq -r '.registry.inputs.nixpkgs.from.narHash' .flox/env/manifest.lock
  assert_success
  assert_output "$NEW_NAR_HASH"
}

@test "update doesn't update an already updated environment" {
  "$FLOX_BIN" init
  
  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_OLD?}" \
   run "$FLOX_BIN" update
  run jq -r '.registry.inputs.nixpkgs.from.narHash' .flox/env/manifest.lock
  assert_success
  assert_output "$OLD_NAR_HASH"
  
  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_OLD?}" \
   run "$FLOX_BIN" update
  assert_success
  assert_output "All inputs are up to date."
  run jq -r '.registry.inputs.nixpkgs.from.narHash' .flox/env/manifest.lock
  assert_success
  assert_output "$OLD_NAR_HASH"
}

@test "update bumps an input but not an already installed package" {
  "$FLOX_BIN" init
  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_OLD?}" \
    "$FLOX_BIN" install hello
  
  # nixpkgs and hello are both locked to the old nixpkgs
  run jq -r '.registry.inputs.nixpkgs.from.narHash' .flox/env/manifest.lock
  assert_success
  assert_output "$OLD_NAR_HASH"
  run jq -r ".packages.\"$NIX_SYSTEM\".hello.input.attrs.narHash" .flox/env/manifest.lock
  assert_success
  assert_output "$OLD_NAR_HASH"
  
  # After an update, nixpkgs is the new nixpkgs, but hello is still from the
  # old one.
  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_NEW?}" \
    "$FLOX_BIN" update
  run jq -r '.registry.inputs.nixpkgs.from.narHash' .flox/env/manifest.lock
  assert_success
  assert_output "$NEW_NAR_HASH"
  run jq -r ".packages.\"$NIX_SYSTEM\".hello.input.attrs.narHash" .flox/env/manifest.lock
  assert_success
  assert_output "$OLD_NAR_HASH"
}

@test "update --global bumps nixpkgs" {
  if [ -f "$GLOBAL_MANIFEST_LOCK" ]; then
    mv "$GLOBAL_MANIFEST_LOCK" "$GLOBAL_MANIFEST_LOCK.bak"
  fi

  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_OLD?}" \
    run "$FLOX_BIN" update --global
  assert_output --partial "Locked all global inputs"
  run jq -r '.registry.inputs.nixpkgs.from.narHash' "$GLOBAL_MANIFEST_LOCK"
  assert_success
  assert_output "$OLD_NAR_HASH"
  
  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_NEW?}" \
    run "$FLOX_BIN" update --global
  assert_success
  assert_output --partial "Updated global"
  assert_output --partial "nixpkgs"
  run jq -r '.registry.inputs.nixpkgs.from.narHash' "$GLOBAL_MANIFEST_LOCK"
  assert_success
  assert_output "$NEW_NAR_HASH"

  if [ -f "$GLOBAL_MANIFEST_LOCK.bak" ]; then
    mv "$GLOBAL_MANIFEST_LOCK.bak" "$GLOBAL_MANIFEST_LOCK"
  fi
}

@test "update --global doesn't update an already updated input" {
  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_OLD?}" \
   run "$FLOX_BIN" update --global
  run jq -r '.registry.inputs.nixpkgs.from.narHash' "$GLOBAL_MANIFEST_LOCK"
  assert_success
  assert_output "$OLD_NAR_HASH"
  
  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_OLD?}" \
   run "$FLOX_BIN" update --global
  assert_success
  assert_output "All inputs are up to date."
  run jq -r '.registry.inputs.nixpkgs.from.narHash' "$GLOBAL_MANIFEST_LOCK"
  assert_success
  assert_output "$OLD_NAR_HASH"
}
