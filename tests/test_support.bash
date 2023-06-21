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
# To customize how
#
# ---------------------------------------------------------------------------- #
#
# NOTE: This file is processed after `setup_suite.bash'.
#
# ---------------------------------------------------------------------------- #

load setup_suite.bash;

# ---------------------------------------------------------------------------- #

require_expect() {
  if ! command -v expect >/dev/null 2>&1; then
    echo "ERROR: expect library needs to be in PATH.";
    return 1;
  fi
}


# ---------------------------------------------------------------------------- #

# `/foo/bar/flox/tests/foo.bats' -> `foo'
setup_test_basename() {
  BATS_TEST_BASENAME="${BATS_TEST_FILENAME##*/}";
  export BATS_TEST_BASENAME="${BATS_TEST_BASENAME%.bats}";
}


# ---------------------------------------------------------------------------- #

# Generate an env name base on the test file's name, setting `TEST_ENVIRONMENT'.
setup_file_envname() {
  setup_test_basename;
  : "${TEST_ENVIRONMENT:=${FLOX_TEST_ENVNAME_PREFIX}${BATS_TEST_BASENAME}}";
  export TEST_ENVIRONMENT;
}


# ---------------------------------------------------------------------------- #

# Build `hello' and root it temporarily so it can be used as an
# install target in various tests.
# This symlink is destroyed by `common_teardown'.
hello_pkg_setup() {
  if [[ -n "${__FT_RAN_HELLO_PKG_SETUP:-}" ]]; then return 0; fi
  export HELLO_LINK="$BATS_SUITE_TMPDIR/gc-roots/hello";
  mkdir -p "${HELLO_LINK%/*}";
  $FLOX_CLI nix build 'nixpkgs#hello' --out-link "$HELLO_LINK";
  HELLO_PACKAGE="$( readlink -f "$HELLO_LINK"; )";
  # Get first 8 characters of store path hash.
  HELLO_FIRST8="${HELLO_PACKAGE#"${NIX_STORE:-/nix/store}/"}";
  HELLO_FIRST8="${HELLO_FIRST8:0:8}";
  export HELLO_PACKAGE HELLO_FIRST8;
  export __FT_RAN_HELLO_PKG_SETUP=:;
}


# ---------------------------------------------------------------------------- #

# Set `XDG_*_HOME' variables to temporary paths.
# This helper should be run after setting `FLOX_TEST_HOME'.
xdg_vars_setup() {
  export XDG_CACHE_HOME="${FLOX_TEST_HOME?}/.cache";
  export XDG_DATA_HOME="${FLOX_TEST_HOME?}/.local/shore";
  export XDG_CONFIG_HOME="${FLOX_TEST_HOME?}/.config";
}


# Copy user's real caches into temporary cache to speed up eval and fetching.
xdg_tmp_setup() {
  xdg_vars_setup;
  if [[ "${__FT_RAN_XDG_TMP_SETUP:-}" = "$XDG_CACHE_HOME" ]]; then return 0; fi
  mkdir -p "$XDG_CACHE_HOME";
  if ! [[ -e "$XDG_CACHE_HOME/nix" ]]; then
    cp -Tpr -- "$REAL_XDG_CACHE_HOME/nix" "$XDG_CACHE_HOME/nix";
  fi
  export __FT_RAN_XDG_TMP_SETUP="$XDG_CACHE_HOME";
}


# ---------------------------------------------------------------------------- #

# This helper should be run after setting `FLOX_TEST_HOME'.
flox_vars_setup() {
  xdg_vars_setup;
  export FLOX_CACHE_HOME="$XDG_CACHE_HOME/flox";
  export FLOX_CONFIG_HOME="$XDG_CONFIG_HOME/flox";
  export FLOX_DATA_HOME="$XDG_DATA_HOME/flox";
  export FLOX_META="$FLOX_CACHE_HOME/meta";
  export FLOX_ENVIRONMENTS="$FLOX_DATA_HOME/environments";
}

# ---------------------------------------------------------------------------- #

# home_setup [suite|file|test]
# ----------------------------
# Set `FLOX_TEST_HOME' to a temporary directory and setup essential files.
# Homedirs can be created "globally" for the entire test suite ( default ), or
# for individual files or single tests by passing an optional argument.
home_setup() {
  case "${1:-suite}" in
    suite) export FLOX_TEST_HOME="${BATS_SUITE_TMPDIR?}/home";                ;;
    file)  export FLOX_TEST_HOME="${BATS_FILE_TMPDIR?}/home";                 ;;
    test)  export FLOX_TEST_HOME="${BATS_TEST_TMPDIR?}/home";                 ;;
    *)     echo "home_setup: Invalid homedir category '${1?}'" >&2; return 1; ;;
  esac
  : "${FLOX_TEST_HOME_STYLE=${1:-suite}}";
  export FLOX_TEST_HOME_STYLE;
  flox_vars_setup;
  export GH_CONFIG_DIR="$XDG_CONFIG_HOME/gh";
  if [[ "${__FT_RAN_HOME_SETUP:-}" = "$FLOX_TEST_HOME" ]]; then return 0; fi
  xdg_tmp_setup;
  export __FT_RAN_HOME_SETUP="$FLOX_TEST_HOME";
}


# ---------------------------------------------------------------------------- #

# Run once for a given `bats' test file.
# This function may be redefined by individual test files, but running
# `common_setup' is the recommended minimum.
common_file_setup() {
  # Generate a `TEST_ENVIRONMENT' name.
  setup_file_envname;
  # Remove any vestiges of previous test runs.
  destroyEnvForce "$TEST_ENVIRONMENT";
  # Setup a homedir associated with this file.
  home_setup "${FLOX_TEST_HOME_STYLE:-suite}";
}

setup_file() { common_file_setup; }


# ---------------------------------------------------------------------------- #

common_file_teardown() {
  # Delete file tmpdir and env unless the user requests to preserve them.
  if [[ -z "${FLOX_TEST_KEEP_TMP:-}" ]]; then
    destroyEnvForce "$TEST_ENVIRONMENT";
    rm -rf "$BATS_FILE_TMPDIR";
  fi
}

teardown_file() { common_file_teardown; }


common_test_teardown() {
  # Delete test tmpdir unless the user requests to preserve them.
  # XXX: We do not attempt to destroy envs here.
  if [[ -z "${FLOX_TEST_KEEP_TMP:-}" ]]; then rm -rf "$BATS_TEST_TMPDIR"; fi
}

teardown() { common_test_teardown; }


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
