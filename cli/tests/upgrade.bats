#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test `flox upgrade`
#
# bats file_tags=upgrade
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
  pushd "$PROJECT_DIR" >/dev/null || return
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.yaml"
}

project_teardown() {
  popd >/dev/null || return
  rm -rf "${PROJECT_DIR?}"
  rm -f "${TMP_MANIFEST_PATH?}"
  unset PROJECT_DIR
  unset MANIFEST_PATH
  unset TMP_MANIFEST_PATH
}

# ---------------------------------------------------------------------------- #

setup() {
  common_test_setup
  setup_isolated_flox
  project_setup
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.yaml"
}
teardown() {
  project_teardown
  common_test_teardown
}

# ---------------------------------------------------------------------------- #
# catalog tests

function hello_response_derivation() {
  yq -r '.then.body' "$GENERATED_DATA/resolve/hello.yaml" | \
    jq -r '.items[].page.packages[0].derivation'
}

function old_hello_response_derivation() {
  yq -r '.then.body' "$GENERATED_DATA/resolve/old_hello.yaml" | \
    jq -r '.items[].page.packages[0].derivation'
}

function hello_response_version() {
  yq -r '.then.body' "$GENERATED_DATA/resolve/hello.yaml" | \
    jq -r '.items[].page.packages[0].version'
}

function old_hello_response_version() {
  yq -r '.then.body' "$GENERATED_DATA/resolve/old_hello.yaml" | \
    jq -r '.items[].page.packages[0].version'
}

# bats test_tags=upgrade:hello
@test "upgrade hello" {
  "$FLOX_BIN" init
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/old_hello.yaml" "$FLOX_BIN" install hello

  old_hello_locked_drv=$(jq -r '.packages.[0].derivation' "$LOCK_PATH")

  assert_equal "$old_hello_locked_drv" "$(old_hello_response_derivation)"

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.yaml" \
    run "$FLOX_BIN" upgrade
  assert_success
  assert_output \
"✅  Upgraded 1 package(s) in 'test':
- hello: $(old_hello_response_version) -> $(hello_response_version)"

  hello_locked_drv=$(jq -r '.packages.[0].derivation' "$LOCK_PATH")
  assert_equal "$hello_locked_drv" "$(hello_response_derivation)"

  assert_not_equal "$old_hello_locked_drv" "$hello_locked_drv"
}

@test "upgrade by group (toplevel)" {
  "$FLOX_BIN" init
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/old_hello.yaml" "$FLOX_BIN" install hello

  old_hello_locked_drv=$(jq -r '.packages.[0].derivation' "$LOCK_PATH")
  assert_equal "$old_hello_locked_drv" "$(old_hello_response_derivation)"

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.yaml" \
    run "$FLOX_BIN" upgrade toplevel
  assert_success
  assert_output \
"✅  Upgraded 1 package(s) in 'test':
- hello: $(old_hello_response_version) -> $(hello_response_version)"

  hello_locked_drv=$(jq -r '.packages.[0].derivation' "$LOCK_PATH")
  assert_equal "$hello_locked_drv" "$(hello_response_derivation)"

  assert_not_equal "$old_hello_locked_drv" "$hello_locked_drv"
}

@test "upgrade by iid" {
  "$FLOX_BIN" init
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/old_hello.yaml" "$FLOX_BIN" install hello

  old_hello_locked_drv=$(jq -r '.packages.[0].derivation' "$LOCK_PATH")
  assert_equal "$old_hello_locked_drv" "$(old_hello_response_derivation)"

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.yaml" \
    run "$FLOX_BIN" upgrade hello
  assert_success
  assert_output \
"✅  Upgraded 1 package(s) in 'test':
- hello: $(old_hello_response_version) -> $(hello_response_version)"

  hello_locked_drv=$(jq -r '.packages.[0].derivation' "$LOCK_PATH")
  assert_equal "$hello_locked_drv" "$(hello_response_derivation)"

  assert_not_equal "$old_hello_locked_drv" "$hello_locked_drv"
}

@test "upgrade errors on iid in group with other packages" {
  "$FLOX_BIN" init
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/curl_hello.yaml" "$FLOX_BIN" install curl hello

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.yaml" \
    run "$FLOX_BIN" upgrade hello
  assert_failure
  assert_line "❌ ERROR: 'hello' is a package in the group 'toplevel' with multiple packages."
}

# bats test_tags=upgrade:page-not-upgraded
@test "page changes should not be considered an upgrade" {
  "$FLOX_BIN" init
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/curl_hello.yaml" \
    "$FLOX_BIN" install curl hello
  prev_lock=$(jq --sort-keys . "$LOCK_PATH")

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/curl_hello_bumped_revs.yaml" \
    run "$FLOX_BIN" upgrade
  assert_success
  assert_output "No upgrades available for packages in 'test'."

  curr_lock=$(jq --sort-keys . "$LOCK_PATH")

  run diff -u <(echo "$prev_lock") <(echo "$curr_lock")
  assert_success
}

# bats test_tags=upgrade:dry-run
@test "'upgrade --dry-run' does not update the lockfile" {
  "$FLOX_BIN" init
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/old_hello.yaml" "$FLOX_BIN" install hello

  old_hello_locked_drv=$(jq -r '.packages.[0].derivation' "$LOCK_PATH")

  assert_equal "$old_hello_locked_drv" "$(old_hello_response_derivation)"

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.yaml" \
    run "$FLOX_BIN" upgrade --dry-run
  assert_success
  assert_output \
"Dry run: Upgrades available for 1 package(s) in 'test':
- hello: $(old_hello_response_version) -> $(hello_response_version)

To apply these changes, run upgrade without the '--dry-run' flag."

  hello_locked_drv=$(jq -r '.packages.[0].derivation' "$LOCK_PATH")
  assert_equal "$hello_locked_drv" "$old_hello_locked_drv"
}

@test "upgrade for flake installable" {
  "$FLOX_BIN" init

  run "$FLOX_BIN" install "github:nixos/nixpkgs/$TEST_NIXPKGS_REV_NEW#hello"

  run "$FLOX_BIN" upgrade
  assert_success
  assert_output "No upgrades available for packages in 'test'."

  new_version="$(jq -r '.packages[0]."version"' "$LOCK_PATH")"
  old_version="2.10.0"

  jq_edit "$LOCK_PATH" '.packages[]."derivation" = "/nix/store/blahblahblah"'
  jq_edit "$LOCK_PATH" '.packages[]."version" = "'"$old_version"'"'

  run "$FLOX_BIN" upgrade
  assert_success

  assert_output \
"✅  Upgraded 1 package(s) in 'test':
- hello: $old_version -> $new_version"
}

# bats test_tags=upgrade:flake:iid
@test "upgrade for flake installable by iid" {
  "$FLOX_BIN" init

  run "$FLOX_BIN" install "github:nixos/nixpkgs/$TEST_NIXPKGS_REV_NEW#hello"

  run "$FLOX_BIN" upgrade hello
  assert_success
  assert_output "No upgrades available for the specified packages in 'test'."
}
