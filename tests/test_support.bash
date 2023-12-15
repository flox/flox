#! /usr/bin/env bash
# ============================================================================ #
#
# Helper utilities shared in common by most tests - particularly
# the routines `setup_*' and `teardown_*'.
#
# By loading this file you will get the common routines as your default; but
# these can be redefined in a particular test file at any point after loading
# and before writing test definitions.
#
# ---------------------------------------------------------------------------- #
#
# NOTE: This file is processed after `setup_suite.bash'.
#
# ---------------------------------------------------------------------------- #

load setup_suite.bash

# ---------------------------------------------------------------------------- #

require_expect() {
  if ! command -v expect > /dev/null 2>&1; then
    echo "ERROR: expect library needs to be in PATH."
    return 1
  fi
}

# ---------------------------------------------------------------------------- #

# `/foo/bar/flox/tests/foo.bats' -> `foo'
setup_test_basename() {
  BATS_TEST_BASENAME="${BATS_TEST_FILENAME##*/}"
  export BATS_TEST_BASENAME="${BATS_TEST_BASENAME%.bats}"
}

# ---------------------------------------------------------------------------- #

# Generate an env name base on the test file's name, setting `TEST_ENVIRONMENT'.
#
# Ex: `test/foo.bats'  ->  `_testing_foo'
setup_file_envname() {
  setup_test_basename
  # Append random number to test environment to avoid collisions when
  # pushing/pulling to floxhub.
  local _random_8digits=$(shuf -i 10000000-99999999 -n 1)
  : "${TEST_ENVIRONMENT:=${FLOX_TEST_ENVNAME_PREFIX}${BATS_TEST_BASENAME}-$_random_8digits}"
  export TEST_ENVIRONMENT
}

# ---------------------------------------------------------------------------- #

# Generate an env name base on the test file's name and the current test number,
# setting `TEST_ENVIRONMENT'.
#
# Ex: `test/foo.bats:@test#4'  ->  `_testing_foo_4'
setup_test_envname() {
  setup_test_basename
  setup_file_envname
  TEST_ENVIRONMENT="$TEST_ENVIRONMENT-$BATS_TEST_NUMBER"
  export TEST_ENVIRONMENT
}

# ---------------------------------------------------------------------------- #

# Build `hello' and root it temporarily so it can be used as an
# install target in various tests.
# This symlink is deleteed by `common_teardown'.
hello_pkg_setup() {
  if [[ -n "${__FT_RAN_HELLO_PKG_SETUP:-}" ]]; then return 0; fi
  export HELLO_LINK="$BATS_SUITE_TMPDIR/gc-roots/hello"
  mkdir -p "${HELLO_LINK%/*}"
  $NIX_BIN --experimental-features "nix-command flakes" build 'nixpkgs#hello' --out-link "$HELLO_LINK"
  HELLO_PACKAGE="$(readlink -f "$HELLO_LINK")"
  # Get first 8 characters of store path hash.
  HELLO_PACKAGE_FIRST8="${HELLO_PACKAGE#"${NIX_STORE:-/nix/store}/"}"
  HELLO_PACKAGE_FIRST8="${HELLO_PACKAGE_FIRST8:0:8}"
  export HELLO_PACKAGE HELLO_PACKAGE_FIRST8
  export __FT_RAN_HELLO_PKG_SETUP=:
}

# ---------------------------------------------------------------------------- #

# common_file_setup [HOME_STYLE ::= (suite|file|test)]
# ----------------------------------------------------
# Run once for a given `bats' test file.
# This function may be redefined by individual test files, but running
# `common_file_setup' is the recommended minimum.
#shellcheck disable=SC2120
common_file_setup() {
  # Generate a `TEST_ENVIRONMENT' name.
  setup_file_envname
  # Remove any vestiges of previous test runs.
  deleteEnvForce "$TEST_ENVIRONMENT"
  # Setup a homedir associated with this file.
  if [[ "${1:-suite}" != test ]]; then home_setup "${1:-suite}"; fi
}

#shellcheck disable=SC2119
setup_file() { common_file_setup; }

# Added for consistency with `teardown' routines.
common_test_setup() { :; }
setup() { common_test_setup; }

# ---------------------------------------------------------------------------- #

common_file_teardown() {
  # Delete file tmpdir and env unless the user requests to preserve them.
  if [[ -z "${FLOX_TEST_KEEP_TMP:-}" ]]; then
    deleteEnvForce "$TEST_ENVIRONMENT"
    rm -rf "$BATS_FILE_TMPDIR"
  fi
  unset FLOX_TEST_HOME
}

teardown_file() { common_file_teardown; }

common_test_teardown() {
  # Delete test tmpdir unless the user requests to preserve them.
  # XXX: We do not attempt to delete envs here.
  if [[ -z "${FLOX_TEST_KEEP_TMP:-}" ]]; then rm -rf "$BATS_TEST_TMPDIR"; fi
}

teardown() { common_test_teardown; }

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
