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

# ---------------------------------------------------------------------------- #

const_setup() {
  if [[ -n "${__PD_RAN_CONST_SETUP:-}" ]]; then return 0; fi

  ENV_BUILDER_NAME="${ENV_BUILDER_NAME:-"flox-env-builder"}"
  ENV_BUILDER_INSTALLABLE="${ENV_BUILDER_INSTALLABLE:-".#flox-env-builder"}"

  export ENV_BUILDER_NAME
  print_var ENV_BUILDER_NAME
  export ENV_BUILDER_INSTALLABLE
  print_var ENV_BUILDER_INSTALLABLE
  export __PD_RAN_CONST_SETUP=:
}

# ---------------------------------------------------------------------------- #

# Locate the `pkgdb' bin to test against.
bin_setup() {
  if [[ -n "${__PD_RAN_ENV_BUILDER_BIN_SETUP:-}" ]]; then return 0; fi
  if [[ -z "${ENV_BUILDER:-}" ]]; then
    if [[ -x "$BATS_TEST_DIRNAME/../bin/$ENV_BUILDER_NAME" ]]; then
      ENV_BUILDER="$BATS_TEST_DIRNAME/../bin/$ENV_BUILDER_NAME"
    elif [[ -x "$BATS_TEST_DIRNAME/result/bin/$ENV_BUILDER_NAME" ]]; then
      ENV_BUILDER="$BATS_TEST_DIRNAME/result/bin/$ENV_BUILDER_NAME"
    else # Build
      (
        cd "$BATS_TEST_DIRNAME" >/dev/null 2>&1 || return 1
        nix build "$ENV_BUILDER_INSTALLABLE"
      )
      ENV_BUILDER="$BATS_TEST_DIRNAME/result/bin/$ENV_BUILDER_NAME"
    fi
  fi

  export ENV_BUILDER
  print_var ENV_BUILDER

  export __PD_RAN_ENV_BUILDER_BIN_SETUP=:
}

# ---------------------------------------------------------------------------- #

# ---------------------------------------------------------------------------- #

# Locate the `pkgdb' bin to test against.
data_setup() {
  if [[ -n "${__PD_RAN_LOCKFILES_SETUP:-}" ]]; then return 0; fi

  LOCKFILES="${LOCKFILES:-"$BATS_TEST_DIRNAME/fixtures/lockfiles"}"

  export LOCKFILES; print_var LOCKFILES

  export __PD_RAN_LOCKFILES_SETUP=:
}

# ---

print_var() { eval echo "  $1: \$$1" >&3; }

# Backup environment variables pointing to "real" system and users paths.
# We sometimes refer to these in order to copy resources from the system into
# our isolated sandboxes.
reals_setup() {
  const_setup
  bin_setup
  data_setup
}

# ---------------------------------------------------------------------------- #

# Lookup system pair recognized by `nix' for this system.
nix_system_setup() {
  # if [[ -z "${NIX_SYSTEM:-}" ]]; then
  #  NIX_SYSTEM="$(nix eval --impure --expr builtins.currentSystem --raw)"
  # fi

  # Lockfiles generated on one system don't contain locks for other systems.
  # The lockfiles in the fixtures directory are generated on darwin
  # so we need to set the NIX_SYSTEM to darwin for compatibility
  # with linux systems.

  NIX_SYSTEM="aarch64-darwin"
  export NIX_SYSTEM; print_var NIX_SYSTEM

}

# ---------------------------------------------------------------------------- #

# Set variables related to locating test resources and misc. bats settings.
misc_vars_setup() {
  if [[ -n "${__PD_RAN_MISC_VARS_SETUP:-}" ]]; then return 0; fi

  # todo: to be used later?

  # export _PKGDB_TEST_SUITE_MODE=:
  # NIXPKGS_REV="e8039594435c68eb4f780f3e9bf3972a7399c4b1"
  # NIXPKGS_REF="github:NixOS/nixpkgs/$NIXPKGS_REV"

  # NIXPKGS_FINGERPRINT="5fde12e3424840cc2752dae09751b09b03f5a33"
  # NIXPKGS_FINGERPRINT="${NIXPKGS_FINGERPRINT}c3ec4de672fc89d236720bdc7" # ???

  # export NIXPKGS_REV; print_var NIXPKGS_REV
  # export NIXPKGS_REF; print_var NIXPKGS_REF
  # export NIXPKGS_FINGERPRINT; print_var NIXPKGS_FINGERPRINT

  export __PD_RAN_MISC_VARS_SETUP=:
}

# ---------------------------------------------------------------------------- #

# Set variables related to locating test resources and misc. bats settings.
env_setup() {
  nix_system_setup
  misc_vars_setup
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
