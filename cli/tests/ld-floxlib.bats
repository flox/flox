#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test that LD_AUDIT and ld-floxlib.so works as expected on Linux only.
#
# This test loads up a flox environment containing the following packages:
# * gcc (to be able to compile the test program)
# * a pinned version of glibc 2.34 from the past
# * patchelf (to modify ELF binaries)
# * nix (a package that is guaranteed to be not available in FHS lib)
# * curl, libarchive (runtime libraries required by libnixmain.so)
#
# It then activates the env to perform two distinct tests:
# 1: load libraries found in $FLOX_ENV_LIBS last
#   - compile the get-glibc-version program (using $FLOX_ENV/{include,lib})
#   - remove its custom RUNPATH and ld interpreter so that it will use the
#     "system" libc
#   - run it having cleared the environment (with `env -i`) and observe the
#     default glibc version, confirm this does NOT match the pinned version
#   - repeat with LD_AUDIT defined and confirm that the version again does
#     not change
#   - repeat with LD_LIBRARY_PATH=$FLOX_ENV_LIBS and confirm that this rolls
#     back glibc to the version installed to the environment
# 2: confirm LD_AUDIT can find missing libraries
#   - compile the get-nix-version program
#   - observe that it can run the compiled program
#   - unset LD_AUDIT and confirm it cannot run the program
#
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# bats file_tags=end2end,ld-floxlib

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
  cp ./ld-floxlib/* "$PROJECT_DIR"
  pushd "$PROJECT_DIR" > /dev/null || return

  # Create environment specifying a pinned nixpkgs revision, although it has no
  # effect (see below).
  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_OLDER?}" $FLOX_BIN init

  # "Update" lock for this one environment to use a pinned nixpkgs revision
  # containing old versions of nix (2.10.3) and glibc (2.34) for use in tests.
  # (Would be preferable if the previous init could honor the revision.)
  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_OLDER?}" $FLOX_BIN update

  # Install packages, including boost, curl and libarchive that are
  # compilation and runtime dependencies of libnixmain.so. Use `flox edit`
  # so that we can bump the priority of the nix package to avoid a path
  # clash with boost.
  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_OLDER?}" \
    $FLOX_BIN edit -f ./manifest.toml
}

project_teardown() {
  popd > /dev/null || return
  unset PROJECT_DIR
  unset PROJECT_NAME
}


# ---------------------------------------------------------------------------- #

setup() {
  if [ $(uname -s) != "Linux" ]; then
    skip "not Linux"
  fi

  common_test_setup
  project_setup
}

teardown() {
  project_teardown
  common_test_teardown
}


# ---------------------------------------------------------------------------- #

@test "test ld-floxlib.so on Linux only" {
  # Revision PKGDB_NIXPKGS_REV_OLDER is expected to provide glibc 2.34.
  # Assert that here before going any further.
  run "$FLOX_BIN" list
  assert_success
  assert_output --partial "glibc: glibc (2.34)"

  # Also assert the environment's loader points to the expected package.
  run "$FLOX_BIN" activate -- \
    find '$FLOX_ENV/lib' -name 'ld-linux-\*.so.\*' -exec realpath {} \\\;
  assert_success
  assert_output --partial -- "-glibc-2.34-210/lib/ld-linux-"

  ### Test 1: load libraries found in $FLOX_ENV_LIB_DIRS last
  $FLOX_BIN activate -- bash ./test-load-library-last.sh
echo yyyy >&3

  ### Test 2: confirm LD_AUDIT can find missing libraries
  # Link against nixmain because that's a library that won't be present on any
  # host system.
  # Build print-nix-version, remove RUNPATH & interpreter
  run "$FLOX_BIN" activate -- bash -eu -o pipefail -c '
    g++ -std=c++17 -o get-nix-version ./get-nix-version.cc  \
        "-I$FLOX_ENV/include" "-L$FLOX_ENV/lib" -lnixmain;
    patchelf --remove-rpath ./get-nix-version;
    patchelf --set-interpreter "$( patchelf --print-interpreter /bin/sh; )"  \
             ./get-nix-version;
    LD_FLOXLIB_DEBUG=1 ./get-nix-version;
  ';
  assert_success
  assert_output --partial "testing (Nix) 2.10.3"

  ### Test 3: confirm binary cannot find missing libraries without LD_AUDIT
  # Note run with "run -127" to silence the 127 "Command not found" error code
  # warning that bats will display by default when it attempts to launch a
  # command that fails to run because it cannot load its libraries.
  run -127 "$FLOX_BIN" activate -- bash -exc \
    '"env -i LD_DEBUG=libs ./get-nix-version"'
  assert_failure
}


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
