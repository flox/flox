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
  export FLOX_FEATURES_USE_CATALOG=true
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.json"
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

@test "flox activate works with npm" {
  export FLOX_FEATURES_USE_CATALOG=false
  cp -r "$INPUT_DATA/init/node/common/." .
  cp -r "$INPUT_DATA/init/node/npm/." .
  # Files copied from the store are read-only
  chmod -R +w .

  run "$FLOX_BIN" init --auto-setup
  assert_output --partial "'nodejs' installed"
  run "$FLOX_BIN" activate -- npm run start
  assert_output --partial "86400000"
}

@test "flox activate works with yarn" {
  export FLOX_FEATURES_USE_CATALOG=false
  cp -r "$INPUT_DATA/init/node/common/." .
  cp -r "$INPUT_DATA/init/node/yarn/." .
  # Files copied from the store are read-only
  chmod -R +w .

  run "$FLOX_BIN" init --auto-setup
  assert_output --partial "'yarn' installed"
  refute_output "nodejs"
  run "$FLOX_BIN" activate -- yarn run start
  assert_output --partial "86400000"
}

@test "install krb5 with node" {
  export FLOX_FEATURES_USE_CATALOG=false
  "$FLOX_BIN" init

  # install a bunch of dependencies needed by npm install krb5 (except for
  # krb5, which is installed below)
  case "$NIX_SYSTEM" in
    *-linux)
      MANIFEST_CONTENT="$(cat << "EOF"
        [install]
        nodejs.pkg-path = "nodejs"
        python3.pkg-path = "python3"
        make.pkg-path = "gnumake"

        # Linux only
        gcc.pkg-path = "gcc"
EOF
  )"
      echo "$MANIFEST_CONTENT" | "$FLOX_BIN" edit -f -

      # Ensure we're getting krb5 from the flox package by first checking
      # installation fails
      run ! "$FLOX_BIN" activate -- bash "$TESTS_DIR/node/krb5.sh"

      "$FLOX_BIN" install krb5

      "$FLOX_BIN" activate -- bash "$TESTS_DIR/node/krb5.sh"
      ;;
    *-darwin)
      MANIFEST_CONTENT="$(cat << "EOF"
        [install]
        nodejs.pkg-path = "nodejs"
        python3.pkg-path = "python3"
        make.pkg-path = "gnumake"

        # darwin only
        clang.pkg-path = "clang"
        cctools = { pkg-path = "darwin.cctools", priority = 6 }

        # TODO: these are only necessary because of how we handle CPATH in
        # activate
        libcxx.pkg-path = "libcxx"
        libcxxabi.pkg-path = "libcxxabi"
EOF
  )"
      echo "$MANIFEST_CONTENT" | "$FLOX_BIN" edit -f -

      # Ensure we're getting krb5 from the flox package by first checking
      # installation fails
      run ! "$FLOX_BIN" activate -- bash -c 'CPATH="$FLOX_ENV/include/c++/v1:$CPATH" . "$TESTS_DIR/node/krb5.sh"'

      "$FLOX_BIN" install krb5

      # TODO: fix CPATH in activate
      "$FLOX_BIN" activate -- bash -c 'CPATH="$FLOX_ENV/include/c++/v1:$CPATH" . "$TESTS_DIR/node/krb5.sh"'
      ;;
    *)
      echo "unsupported system: $NIX_SYSTEM"
      return 1
      ;;
  esac
}

# ---------------------------------------------------------------------------- #
# catalog tests

# bats test_tags=catalog
@test "catalog: flox activate works with npm" {
  cp -r "$INPUT_DATA/init/node/common/." .
  cp -r "$INPUT_DATA/init/node/npm/." .
  # Files copied from the store are read-only
  chmod -R +w .

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/init/node_npm.json" \
    run "$FLOX_BIN" init --auto-setup
  assert_output --partial "'nodejs' installed"
  run "$FLOX_BIN" activate -- npm run start
  assert_output --partial "86400000"
}

# bats test_tags=catalog
@test "catalog: flox activate works with yarn" {
  cp -r "$INPUT_DATA/init/node/common/." .
  cp -r "$INPUT_DATA/init/node/yarn/." .
  # Files copied from the store are read-only
  chmod -R +w .

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/init/node_yarn.json" \
    run "$FLOX_BIN" init --auto-setup
  assert_output --partial "'yarn' installed"
  refute_output "nodejs"
  run "$FLOX_BIN" activate -- yarn run start
  assert_output --partial "86400000"
}

# bats test_tags=catalog
@test "catalog: install krb5 with node" {
  "$FLOX_BIN" init

  cat "$GENERATED_DATA/envs/krb5_prereqs/manifest.toml" | _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/envs/krb5_prereqs/krb5_prereqs.json" "$FLOX_BIN" edit -f -

  # With dependencies installed, we can now install krb5 and run system-specific
  # checks.
  case "$NIX_SYSTEM" in
    *-linux)
      # Ensure we're getting krb5 from the flox package by first checking
      # installation fails
      run ! "$FLOX_BIN" activate -- bash "$TESTS_DIR/init/node/krb5.sh"

      _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/krb5_after_prereqs_installed.json" \
        "$FLOX_BIN" install krb5

      "$FLOX_BIN" activate -- bash "$TESTS_DIR/node/krb5.sh"
      ;;
    *-darwin)
      # Ensure we're getting krb5 from the flox package by first checking
      # installation fails
      run ! "$FLOX_BIN" activate -- bash -c 'CPATH="$FLOX_ENV/include/c++/v1:$CPATH" . "$TESTS_DIR/init/node/krb5.sh"'

      _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/krb5_after_prereqs_installed.json" \
          "$FLOX_BIN" install krb5

      # TODO: fix CPATH in activate
      "$FLOX_BIN" activate -- bash -c 'CPATH="$FLOX_ENV/include/c++/v1:$CPATH" . "$TESTS_DIR/node/krb5.sh"'
      ;;
    *)
      echo "unsupported system: $NIX_SYSTEM"
      return 1
      ;;
  esac
}
