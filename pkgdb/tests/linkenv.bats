#! /usr/bin/env bats
# --------------------------------------------------------------------------- #
#
# @file tests/linkenv.bats
#
# @brief Test linking environments from buildenv.
#
# These tests only check the build segment,
# they do not check the resolution of manifests,
# nor the activation of the resulting environments.
# Such tests are found in `pkgdb` and `flox` respectively.
#
# --------------------------------------------------------------------------- #

# bats file_tags=link-env

load setup_suite.bash

# --------------------------------------------------------------------------- #

setup_file() {
  : "${CAT:=cat}"
  : "${TEST:=test}"
  : "${MKDIR:=mkdir}"
  export CAT TEST MKDIR
  export LOCKFILES="${BATS_FILE_TMPDIR?}/lockfiles"

  # Always use a consistent `nixpkgs' input.
  export _PKGDB_GA_REGISTRY_REF_OR_REV="${NIXPKGS_REV?}"

  # Generate lockfiles
  for dir in "${TESTS_DIR?}"/data/buildenv/lockfiles/*; do
    if $TEST -d "$dir"; then
      _lockfile="${LOCKFILES?}/${dir##*/}/manifest.lock"
      $MKDIR -p "${_lockfile%/*}"
      ${PKGDB_BIN?} manifest lock --ga-registry --manifest \
        "$dir/manifest.toml" > "$_lockfile"
    fi
  done
}

# ---------------------------------------------------------------------------- #

# bats test_tags=matches
@test "Linked environment matches the built environment" {
  run "$PKGDB_BIN" buildenv \
    "$LOCKFILES/single-package/manifest.lock"
  assert_success
  store_path=$(echo "$output" | jq -er '.store_path')

  run "$PKGDB_BIN" linkenv \
    --out-link "${BATS_TEST_TMPDIR}/env" \
    --store-path "${store_path}"
  assert_success
  assert_output --partial "${store_path}"

  assert "$TEST" $(readlink "${BATS_TEST_TMPDIR}/env") == "${store_path}"

  # Can be run multiple times.
  run "$PKGDB_BIN" linkenv \
    --out-link "${BATS_TEST_TMPDIR}/env" \
    --store-path "${store_path}"
  assert_success
  assert_output --partial "${store_path}"
}

# bats test_tags=store-path
@test "Link fails when store-path doesn't exist" {
  run "$PKGDB_BIN" linkenv \
    --out-link "${BATS_TEST_TMPDIR}/env" \
    --store-path "/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-foo"
  assert_failure
  assert_output --partial "No such store-path"
}

# bats test_tags=out-link
@test "Link fails when out-link is an existing directory" {
  run "$PKGDB_BIN" buildenv \
    "$LOCKFILES/single-package/manifest.lock"
  assert_success
  store_path=$(echo "$output" | jq -er '.store_path')

  mkdir "${BATS_TEST_TMPDIR}/env"
  run "$PKGDB_BIN" linkenv \
    --out-link "${BATS_TEST_TMPDIR}/env" \
    --store-path "${store_path}"
  assert_failure
  assert_output --partial "cannot create symlink"
}
