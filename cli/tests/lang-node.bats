#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test if node works with flox activate.
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# bats file_tags=end2end

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
  pushd "$PROJECT_DIR" > /dev/null || return
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.yaml"
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
# catalog tests

# bats test_tags=catalog
@test "flox activate works with npm" {
  cp -r "$INPUT_DATA/init/node/npm/." .
  # Files copied from the store are read-only
  chmod -R +w .

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/init/node_npm.yaml" \
    run "$FLOX_BIN" init --auto-setup
  assert_output --partial "'nodejs' installed"
  run "$FLOX_BIN" activate -- npm run start
  assert_output --partial "86400000"
}

# bats test_tags=catalog
@test "auto init matches yarn version to yarn 1.x" {
  cp -r "$INPUT_DATA/init/node/yarn_1x/." .
  # Files copied from the store are read-only
  chmod -R +w .

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/init/yarn_1x.yaml" \
    run "$FLOX_BIN" init --auto-setup
  assert_output --partial "'yarn' installed"
  refute_output "nodejs"
  run "$FLOX_BIN" list
  assert_regex "$output" "yarn: yarn \(.+\)"
  run "$FLOX_BIN" activate -- yarn run start
  assert_output --partial "86400000"
}

# bats test_tags=catalog,init
@test "auto init installs nodejs major version package" {
  cp -r "$INPUT_DATA/init/node/nodejs_20/." .
  chmod -R +w .
  # This test ensures that when a package.json has a version requirement,
  # in this case "20", we give them the corresponding nodejs_* package.
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/init/nodejs_20.yaml" \
    run "$FLOX_BIN" init --auto-setup
  assert_output --partial "'nodejs' installed"
  run "$FLOX_BIN" list
  assert_regex "$output" "nodejs: nodejs_20.*"
}

# bats test_tags=catalog,init
@test "auto init installs nodejs version range package" {
  cp -r "$INPUT_DATA/init/node/nodejs_lt_24/." .
  chmod -R +w .
  # This test ensures that when a package.json has a version requirment,
  # in this case "<24", we give them the nodejs_* package corresponding
  # to the nearly-latest version.
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/init/nodejs_lt_24.yaml" \
    run "$FLOX_BIN" init --auto-setup
  assert_output --partial "'nodejs' installed"
  run "$FLOX_BIN" list
  assert_regex "$output" "nodejs: nodejs_23.*"
}

# bats test_tags=catalog,init
@test "auto init matches yarn version to yarn-berry" {
  cp -r "$INPUT_DATA/init/node/yarn_berry/." .
  chmod -R +w .
  # We specify yarn 4 in `package.json` but this is also equivalent to the
  # default case if there is a `yarn.lock` and no version specified.
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/init/yarn_berry.yaml" \
    run "$FLOX_BIN" init --auto-setup
  assert_output --partial "'yarn' installed"
  run "$FLOX_BIN" list
  assert_regex "$output" "yarn: yarn-berry \(.+\)"
}

# bats test_tags=catalog,krb5
@test "install krb5 with node" {
  "$FLOX_BIN" init

  cat "$GENERATED_DATA/envs/krb5_prereqs/manifest.toml" | _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/envs/krb5_prereqs/krb5_prereqs.yaml" "$FLOX_BIN" edit -f -

  # With dependencies installed, we can now install krb5 and run system-specific
  # checks.
  case "$NIX_SYSTEM" in
    *-linux)
      # Ensure we're getting krb5 from the flox package by first checking
      # installation fails
      # XXX "$TESTS_DIR/init/node/krb5.sh" is not always present so only run
      #     once we have confirmed that it exists, and then expect it to fail.
      if [ -f "$TESTS_DIR/init/node/krb5.sh" ]; then
        run "$FLOX_BIN" activate -- bash "$TESTS_DIR/init/node/krb5.sh"
        assert_failure
      fi

      _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/krb5_after_prereqs_installed.yaml" \
        run "$FLOX_BIN" install krb5
      assert_success

      run "$FLOX_BIN" activate -- bash "$INPUT_DATA/init/node/krb5.sh"
      assert_success
      ;;
    *-darwin)
      # Ensure we're getting krb5 from the flox package by first checking
      # installation fails
      # XXX "$TESTS_DIR/init/node/krb5.sh" is not always present so only run
      #     once we have confirmed that it exists, and then expect it to fail.
      if [ -f "$TESTS_DIR/init/node/krb5.sh" ]; then
        run "$FLOX_BIN" activate -- bash "$TESTS_DIR/init/node/krb5.sh"
        assert_failure
      fi

      _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/krb5_after_prereqs_installed.yaml" \
        run "$FLOX_BIN" install krb5
      assert_success

      run "$FLOX_BIN" activate -- bash "$INPUT_DATA/init/node/krb5.sh"
      assert_success
      ;;
    *)
      echo "unsupported system: $NIX_SYSTEM"
      return 1
      ;;
  esac
}
