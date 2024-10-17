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
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.json"
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
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.json"
}
teardown() {
  project_teardown
  common_test_teardown
}

# create a deprecated v0 environment from prepared data
setup_pkgdb_env() {
  NAME=$1
  shift

  mkdir -p "$PROJECT_DIR/.flox/env"
  cp --no-preserve=mode "$MANUALLY_GENERATED"/empty_v0/* "$PROJECT_DIR/.flox/env"

  echo '{
    "name": "'$NAME'",
    "version": 1
  }' >>"$PROJECT_DIR/.flox/env.json"
}

# ---------------------------------------------------------------------------- #
# catalog tests

@test "upgrade hello" {
  "$FLOX_BIN" init
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/old_hello.json" "$FLOX_BIN" install hello

  old_hello_response_drv=$(jq -r '.[0].[0].page.packages[0].derivation' "$GENERATED_DATA/resolve/old_hello.json")
  old_hello_locked_drv=$(jq -r '.packages.[0].derivation' "$LOCK_PATH")
  assert_equal "$old_hello_locked_drv" "$old_hello_response_drv"

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json" \
    run "$FLOX_BIN" upgrade
  assert_success
  assert_output "⬆️  Upgraded 'hello' in environment 'test'."

  hello_response_drv=$(jq -r '.[0].[0].page.packages[0].derivation' "$GENERATED_DATA/resolve/hello.json")
  hello_locked_drv=$(jq -r '.packages.[0].derivation' "$LOCK_PATH")
  assert_equal "$hello_locked_drv" "$hello_response_drv"

  assert_not_equal "$old_hello_locked_drv" "$hello_locked_drv"
}

@test "upgrade by group" {
  "$FLOX_BIN" init
  cp "$MANIFEST_PATH" "$TMP_MANIFEST_PATH"
  tomlq -i -t '.install.hello."pkg-path" = "hello"' "$TMP_MANIFEST_PATH"
  tomlq -i -t '.install.hello."pkg-group" = "blue"' "$TMP_MANIFEST_PATH"
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/old_hello.json" "$FLOX_BIN" edit -f "$TMP_MANIFEST_PATH"

  old_hello_response_drv=$(jq -r '.[0].[0].page.packages[0].derivation' "$GENERATED_DATA/resolve/old_hello.json")
  old_hello_locked_drv=$(jq -r '.packages.[0].derivation' "$LOCK_PATH")
  assert_equal "$old_hello_locked_drv" "$old_hello_response_drv"

  # add the package group

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json" \
    run "$FLOX_BIN" upgrade blue
  assert_success
  assert_output "⬆️  Upgraded 'hello' in environment 'test'."

  hello_response_drv=$(jq -r '.[0].[0].page.packages[0].derivation' "$GENERATED_DATA/resolve/hello.json")
  hello_locked_drv=$(jq -r '.packages.[0].derivation' "$LOCK_PATH")
  assert_equal "$hello_locked_drv" "$hello_response_drv"

  assert_not_equal "$old_hello_locked_drv" "$hello_locked_drv"
}

@test "upgrade toplevel group" {
  "$FLOX_BIN" init
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/old_hello.json" "$FLOX_BIN" install hello

  old_hello_response_drv=$(jq -r '.[0].[0].page.packages[0].derivation' "$GENERATED_DATA/resolve/old_hello.json")
  old_hello_locked_drv=$(jq -r '.packages.[0].derivation' "$LOCK_PATH")
  assert_equal "$old_hello_locked_drv" "$old_hello_response_drv"

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json" \
    run "$FLOX_BIN" upgrade toplevel
  assert_success
  assert_output "⬆️  Upgraded 'hello' in environment 'test'."

  hello_response_drv=$(jq -r '.[0].[0].page.packages[0].derivation' "$GENERATED_DATA/resolve/hello.json")
  hello_locked_drv=$(jq -r '.packages.[0].derivation' "$LOCK_PATH")
  assert_equal "$hello_locked_drv" "$hello_response_drv"

  assert_not_equal "$old_hello_locked_drv" "$hello_locked_drv"
}

@test "upgrade by iid" {
  "$FLOX_BIN" init
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/old_hello.json" "$FLOX_BIN" install hello

  old_hello_response_drv=$(jq -r '.[0].[0].page.packages[0].derivation' "$GENERATED_DATA/resolve/old_hello.json")
  old_hello_locked_drv=$(jq -r '.packages.[0].derivation' "$LOCK_PATH")
  assert_equal "$old_hello_locked_drv" "$old_hello_response_drv"

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json" \
    run "$FLOX_BIN" upgrade hello
  assert_success
  assert_output "⬆️  Upgraded 'hello' in environment 'test'."

  hello_response_drv=$(jq -r '.[0].[0].page.packages[0].derivation' "$GENERATED_DATA/resolve/hello.json")
  hello_locked_drv=$(jq -r '.packages.[0].derivation' "$LOCK_PATH")
  assert_equal "$hello_locked_drv" "$hello_response_drv"

  assert_not_equal "$old_hello_locked_drv" "$hello_locked_drv"
}

@test "upgrade errors on iid in group with other packages" {
  "$FLOX_BIN" init
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/curl_hello.json" "$FLOX_BIN" install curl hello

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.json" \
    run "$FLOX_BIN" upgrade hello
  assert_failure
  assert_line "❌ ERROR: 'hello' is a package in the group 'toplevel' with multiple packages."
}

@test "check confirmation when all packages are up to date" {
  "$FLOX_BIN" init
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/curl_hello.json" "$FLOX_BIN" install curl hello

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/curl_hello.json" \
    run "$FLOX_BIN" upgrade
  assert_success
  assert_output "ℹ️  No packages need to be upgraded in environment 'test'."
}

@test "page changes should not be considered an upgrade" {
  "$FLOX_BIN" init
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/curl_hello.json" \
    "$FLOX_BIN" install curl hello
  prev_lock_hash=$(jq --sort-keys --compact-output . "$LOCK_PATH" | sha256sum)

  # Update the page and revision but keep the same derivations.
  # This would fail to rebuild because the revs are faked.
  BUMPED_REVS_RESPONE="curl_hello_bumped_revs.json"
  jq '.[0][0].page |= (
    (.page | .+ 123) as $newpage |
    .page = $newpage |
    .packages |= map(
      (.rev | .[0:-8] + "deadbeef") as $newrev |
      .rev_count = $newpage |
      .rev = $newrev |
      .locked_url |= sub("rev=.*"; "rev=" + $newrev)
    ))' \
    "$GENERATED_DATA/resolve/curl_hello.json" \
    > "$BUMPED_REVS_RESPONE"
  _FLOX_USE_CATALOG_MOCK="$BUMPED_REVS_RESPONE" \
    run "$FLOX_BIN" upgrade
  assert_success
  assert_output "ℹ️  No packages need to be upgraded in environment 'test'."

  curr_lock_hash=$(jq --sort-keys --compact-output . "$LOCK_PATH" | sha256sum)
  assert_equal "$curr_lock_hash" "$prev_lock_hash"
}

@test "upgrade for flake installable" {
  "$FLOX_BIN" init

  MANIFEST_CONTENTS="$(cat << "EOF"
  version = 1

  [install]
  hello.flake = "github:NixOS/nixpkgs#hello"
EOF
  )"

  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f -

  run "$FLOX_BIN" upgrade
  assert_success
  assert_output --partial "No packages need to be upgraded"

  jq_edit "$LOCK_PATH" '.packages[]."derivation" = "/nix/store/blahblahblah"'

  run "$FLOX_BIN" upgrade
  assert_success
  assert_output "⬆️  Upgraded 'hello' in environment 'test'."
}
