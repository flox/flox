#! /usr/bin/env bash
# ============================================================================ #
#
# Early setup routines used to initialize the test suite.
# This is run once every time `bats' is invoked, but is never rerun between
# individual files or tests.
#
# ---------------------------------------------------------------------------- #

bats_load_library bats-support;
bats_load_library bats-assert;
bats_require_minimum_version '1.5.0';


# ---------------------------------------------------------------------------- #

# Locate repository root.
repo_root_setup() {
  if [[ -z "${REPO_ROOT:-}" ]]; then
    if [[ -d "$PWD/.git" ]] && [[ -d "$PWD/tests" ]]; then
      REPO_ROOT="$PWD";
    else
      REPO_ROOT="$( git rev-parse --show-toplevel||:; )";
    fi
    if [[ -z "$REPO_ROOT" ]] && [[ -d "$PWD/tests" ]]; then
      REPO_ROOT="$PWD";
    fi
  fi
  export REPO_ROOT;
}


# ---------------------------------------------------------------------------- #

# Locate the directory containing test resources.
tests_dir_setup() {
  if [[ -n "${__PD_RAN_TESTS_DIR_SETUP:-}" ]]; then return 0; fi
  repo_root_setup;
  if [[ -z "${TEST_DIR:-}" ]]; then
    case "${BATS_TEST_DIRNAME:-}" in
      */tests) TESTS_DIR="$( readlink -f "$BATS_TEST_DIRNAME"; )"; ;;
      *)       TESTS_DIR="$REPO_ROOT/tests";                       ;;
    esac
    if ! [[ -d "$TESTS_DIR" ]]; then
      echo "tests_dir_setup: \`TESTS_DIR' must be a directory" >&2;
      return 1;
    fi
  fi
  export TESTS_DIR;
  export __PD_RAN_TESTS_DIR_SETUP=:;
}


# ---------------------------------------------------------------------------- #

# Locate the `pkgdb' bin to test against.
pkgdb_bin_setup() {
  if [[ -n "${__PD_RAN_PKGDB_BIN_SETUP:-}" ]]; then return 0; fi
  if [[ -z "${PKGDB:-}" ]]; then
    repo_root_setup;
    if [[ -x "$REPO_ROOT/bin/pkgdb" ]]; then
      PKGDB="$REPO_ROOT/bin/pkgdb";
    elif [[ -x "$REPO_ROOT/result/bin/pkgdb" ]]; then
      PKGDB="$REPO_ROOT/result/bin/pkgdb";
    else  # Build
      (
        cd "$REPO_ROOT" >/dev/null 2>&1||return 1;
        nix develop -c make -j8;
      );
      PKGDB="$REPO_ROOT/bin/pkgdb";
    fi
  fi
  export PKGDB;
  export __PD_RAN_PKGDB_BIN_SETUP=:;
}


# ---------------------------------------------------------------------------- #


print_var() { eval echo "  $1: \$$1"; }

# Backup environment variables pointing to "real" system and users paths.
# We sometimes refer to these in order to copy resources from the system into
# our isolated sandboxes.
reals_setup() {
  repo_root_setup;
  tests_dir_setup;
  pkgdb_bin_setup;
  {
    print_var REPO_ROOT;
    print_var TESTS_DIR;
    print_var PKGDB;
  } >&3;
}


# ---------------------------------------------------------------------------- #

# Lookup system pair recognized by `nix' for this system.
nix_system_setup() {
  if [[ -z "${NIX_SYSTEM:-}" ]]; then
    NIX_SYSTEM="$( nix eval --impure --expr builtins.currentSystem --raw; )";
  fi
  export NIX_SYSTEM;
}


# ---------------------------------------------------------------------------- #

# Set variables related to locating test resources and misc. bats settings.
misc_vars_setup() {
  if [[ -n "${__PD_RAN_MISC_VARS_SETUP:-}" ]]; then return 0; fi

  export _PKGDB_TEST_SUITE_MODE=:;

  NIXPKGS_REV="e8039594435c68eb4f780f3e9bf3972a7399c4b1";
  NIXPKGS_REF="github:NixOS/nixpkgs/$NIXPKGS_REV";

  NIXPKGS_FINGERPRINT="5fde12e3424840cc2752dae09751b09b03f5a33"
  NIXPKGS_FINGERPRINT="${NIXPKGS_FINGERPRINT}c3ec4de672fc89d236720bdc7";

  export NIXPKGS_REV NIXPKGS_REF NIXPKGS_FINGERPRINT;

  export __PD_RAN_MISC_VARS_SETUP=:;
}


# ---------------------------------------------------------------------------- #

# Set variables related to locating test resources and misc. bats settings.
env_setup() {
  nix_system_setup;
  misc_vars_setup;
  {
    print_var NIX_SYSTEM;
    print_var NIXPKGS_REV;
    print_var NIXPKGS_REF;
  } >&3;
}


# ---------------------------------------------------------------------------- #

common_suite_setup() {
  reals_setup;
  env_setup;
}


# Recognized by `bats'.
setup_suite() { common_suite_setup; }


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
