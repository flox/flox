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
  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" > /dev/null || return
}

project_teardown() {
  popd > /dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
}

# ---------------------------------------------------------------------------- #

setup() {
  common_test_setup
  setup_isolated_flox
  project_setup

  # update is only supported with pkgdb
  export FLOX_FEATURES_USE_CATALOG=false
}
teardown() {
  project_teardown
  common_test_teardown
}

@test "update scrapes input" {
  database_path=$("$PKGDB_BIN" get db "$PKGDB_NIXPKGS_REF_OLD")
  # As far as I can tell, scraping isn't too expensive since we have an eval
  # cache.
  rm -f "$database_path"

  "$FLOX_BIN" init

  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_OLD?}" \
    run "$FLOX_BIN" update
  run jq -r '.registry.inputs.nixpkgs.from.narHash' .flox/env/manifest.lock
  assert_success
  assert_output "$PKGDB_NIXPKGS_NAR_HASH_OLD"

  [[ -f "$database_path" ]]
}

@test "update bumps nixpkgs" {
  "$FLOX_BIN" init

  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_OLD?}" \
    run "$FLOX_BIN" update
  assert_output --partial "Locked input"
  run jq -r '.registry.inputs.nixpkgs.from.narHash' .flox/env/manifest.lock
  assert_success
  assert_output "$PKGDB_NIXPKGS_NAR_HASH_OLD"

  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_NEW?}" \
    run "$FLOX_BIN" update
  assert_success
  assert_output --partial "Updated input 'nixpkgs'"
  run jq -r '.registry.inputs.nixpkgs.from.narHash' .flox/env/manifest.lock
  assert_success
  assert_output "$PKGDB_NIXPKGS_NAR_HASH_NEW"
}

@test "update doesn't update an already updated environment" {
  "$FLOX_BIN" init

  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_OLD?}" \
    run "$FLOX_BIN" update
  run jq -r '.registry.inputs.nixpkgs.from.narHash' .flox/env/manifest.lock
  assert_success
  assert_output "$PKGDB_NIXPKGS_NAR_HASH_OLD"

  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_OLD?}" \
    run "$FLOX_BIN" update
  assert_success
  assert_output --partial "All inputs are up-to-date"
  run jq -r '.registry.inputs.nixpkgs.from.narHash' .flox/env/manifest.lock
  assert_success
  assert_output "$PKGDB_NIXPKGS_NAR_HASH_OLD"
}

@test "update bumps an input but not an already installed package" {
  rm -f "$GLOBAL_MANIFEST_LOCK"

  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_OLD?}" \
    "$FLOX_BIN" init
  "$FLOX_BIN" install hello

  # nixpkgs and hello are both locked to the old nixpkgs
  run jq -r '.registry.inputs.nixpkgs.from.narHash' .flox/env/manifest.lock
  assert_success
  assert_output "$PKGDB_NIXPKGS_NAR_HASH_OLD"
  run jq -r ".packages.\"$NIX_SYSTEM\".hello.input.attrs.narHash" .flox/env/manifest.lock
  assert_success
  assert_output "$PKGDB_NIXPKGS_NAR_HASH_OLD"

  # After an update, nixpkgs is the new nixpkgs, but hello is still from the
  # old one.
  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_NEW?}" \
    "$FLOX_BIN" update
  run jq -r '.registry.inputs.nixpkgs.from.narHash' .flox/env/manifest.lock
  assert_success
  assert_output "$PKGDB_NIXPKGS_NAR_HASH_NEW"
  run jq -r ".packages.\"$NIX_SYSTEM\".hello.input.attrs.narHash" .flox/env/manifest.lock
  assert_success
  assert_output "$PKGDB_NIXPKGS_NAR_HASH_OLD"
}

@test "update --global bumps nixpkgs" {
  if [ -f "$GLOBAL_MANIFEST_LOCK" ]; then
    mv "$GLOBAL_MANIFEST_LOCK" "$GLOBAL_MANIFEST_LOCK.bak"
  fi

  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_OLD?}" \
    run "$FLOX_BIN" update --global
  assert_output --partial "Locked global input 'nixpkgs'"
  run jq -r '.registry.inputs.nixpkgs.from.narHash' "$GLOBAL_MANIFEST_LOCK"
  assert_success
  assert_output "$PKGDB_NIXPKGS_NAR_HASH_OLD"

  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_NEW?}" \
    run "$FLOX_BIN" update --global
  assert_success
  assert_output --partial "Updated global"
  assert_output --partial "nixpkgs"
  run jq -r '.registry.inputs.nixpkgs.from.narHash' "$GLOBAL_MANIFEST_LOCK"
  assert_success
  assert_output "$PKGDB_NIXPKGS_NAR_HASH_NEW"

  if [ -f "$GLOBAL_MANIFEST_LOCK.bak" ]; then
    mv "$GLOBAL_MANIFEST_LOCK.bak" "$GLOBAL_MANIFEST_LOCK"
  fi
}

@test "update --global doesn't update an already updated input" {
  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_OLD?}" \
    run "$FLOX_BIN" update --global
  run jq -r '.registry.inputs.nixpkgs.from.narHash' "$GLOBAL_MANIFEST_LOCK"
  assert_success
  assert_output "$PKGDB_NIXPKGS_NAR_HASH_OLD"

  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_OLD?}" \
    run "$FLOX_BIN" update --global
  assert_success
  assert_output --partial "All global inputs are up-to-date."
  run jq -r '.registry.inputs.nixpkgs.from.narHash' "$GLOBAL_MANIFEST_LOCK"
  assert_success
  assert_output "$PKGDB_NIXPKGS_NAR_HASH_OLD"
}
