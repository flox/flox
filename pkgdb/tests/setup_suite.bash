#! /usr/bin/env bash
# ============================================================================ #
#
# Early setup routines used to initialize the test suite.
# This is run once every time `bats' is invoked, but is never rerun between
# individual files or tests.
#
# ---------------------------------------------------------------------------- #

bats_load_library bats-support
bats_load_library bats-assert
bats_require_minimum_version '1.5.0'

# ---------------------------------------------------------------------------- #

# Locate repository root.
repo_root_setup() {
  if [[ -z "${REPO_ROOT:-}" ]]; then
    if [[ -d "$PWD/../.git" ]]; then
      REPO_ROOT="${PWD%/*}"
    else
      REPO_ROOT="$(git rev-parse --show-toplevel || :)"
    fi
    if [[ -z "$REPO_ROOT" ]] && [[ -d "$PWD/src/pkgdb/read.cc" ]]; then
      REPO_ROOT="${PWD%/*}"
    fi
  fi
  export REPO_ROOT
}

# ---------------------------------------------------------------------------- #

# Locate the directory containing test resources.
tests_dir_setup() {
  if [[ -n "${__PD_RAN_TESTS_DIR_SETUP:-}" ]]; then return 0; fi
  repo_root_setup
  if [[ -z "${TESTS_DIR:-}" ]]; then
    case "${BATS_TEST_DIRNAME:-}" in
      */tests) TESTS_DIR="$(readlink -f "$BATS_TEST_DIRNAME")" ;;
      *) TESTS_DIR="$REPO_ROOT/pkgdb/tests" ;;
    esac
    if ! [[ -d "$TESTS_DIR" ]]; then
      echo "tests_dir_setup: \`TESTS_DIR' must be a directory" >&2
      return 1
    fi
  fi
  export TESTS_DIR
  export __PD_RAN_TESTS_DIR_SETUP=:
}

# ---------------------------------------------------------------------------- #

print_var() { eval echo "  $1: \$$1"; }

# Backup environment variables pointing to "real" system and users paths.
# We sometimes refer to these in order to copy resources from the system into
# our isolated sandboxes.
reals_setup() {
  repo_root_setup
  tests_dir_setup
  {
    print_var REPO_ROOT
    print_var TESTS_DIR
    print_var PKGDB_BIN
  } >&3
}

# ---------------------------------------------------------------------------- #

# Lookup system pair recognized by `nix' for this system.
nix_system_setup() {
  if [[ -z "${NIX_SYSTEM:-}" ]]; then
    NIX_SYSTEM="$(nix eval --impure --expr builtins.currentSystem --raw)"
  fi
  export NIX_SYSTEM
}

# ---------------------------------------------------------------------------- #

# Set variables related to locating test resources and misc. bats settings.
misc_vars_setup() {
  if [[ -n "${__PD_RAN_MISC_VARS_SETUP:-}" ]]; then return 0; fi

  export _PKGDB_TEST_SUITE_MODE=:

  # Incomplete notes on steps needed to bump:
  # _PKGDB_GA_REGISTRY_REF_OR_REV=$NIXPKGS_REV \
  #   pkgdb manifest lock --ga-registry --manifest pkgdb/tests/harnesses/proj1/manifest.toml \
  #   | jq > pkgdb/tests/harnesses/proj1/manifest.lock
  NIXPKGS_REV="ab5fd150146dcfe41fda501134e6503932cc8dfd"
  NIXPKGS_REF="github:NixOS/nixpkgs/$NIXPKGS_REV"

  NIXPKGS_FINGERPRINT="9bb3d4c033fbad8efb5e28ffcd1d70383e0c5bb"
  NIXPKGS_FINGERPRINT="${NIXPKGS_FINGERPRINT}cb7cc5c526b824524467b19b9"

  export NIXPKGS_REV NIXPKGS_REF NIXPKGS_FINGERPRINT

  NODEJS_VERSION="18.18.2"
  export NODEJS_VERSION

  NIXPKGS_REV_OLD="e8039594435c68eb4f780f3e9bf3972a7399c4b1"
  export NIXPKGS_REV_OLD
  NODEJS_VERSION_OLD="18.16.0"
  export NODEJS_VERSION_OLD

  # See reasons for choosing this rev in cli tests
  # Incomplete notes on steps needed to bump:
  # _PKGDB_GA_REGISTRY_REF_OR_REV=$NIXPKGS_REV_OLDER \
  #   pkgdb manifest lock --ga-registry --manifest pkgdb/tests/harnesses/proj1/manifest.toml \
  #   | jq > pkgdb/tests/harnesses/proj1/manifest_old.lock
  NIXPKGS_REV_OLDER="bc01a2be500c10f1507dcc8e98c9f5bd72c02aa3"
  export NIXPKGS_REV_OLDER
  NODEJS_VERSION_OLDEST="16.16.0"
  export NODEJS_VERSION_OLDEST

  # Default version for `flox-nixpkgs` inputs.
  # NOTE: Keep in line with `../src/registry/wrapped-nixpkgs-input.cc`
  export FLOX_NIXPKGS_VERSION="0";

  export __PD_RAN_MISC_VARS_SETUP=:
}

# ---------------------------------------------------------------------------- #

# Set variables related to locating test resources and misc. bats settings.
env_setup() {
  nix_system_setup
  misc_vars_setup
  {
    print_var NIX_SYSTEM
    print_var NIXPKGS_REV
    print_var NIXPKGS_REF
  } >&3
}

# ---------------------------------------------------------------------------- #

common_suite_setup() {
  reals_setup
  env_setup
}

# Recognized by `bats'.
setup_suite() { common_suite_setup; }

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
