#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test that LD_AUDIT and ld-floxlib.so works as expected on Linux only.
#
# This test loads up a flox environment containing the following packages as
# installed by tests/ld-floxlib.bats:
# * gcc-wrapped (to be able to compile the test program)
# * a pinned version of glibc from the past
# * giflib (a package that is presumed to be not available by default)
#
# It then activates the env to perform two distinct tests:
# 1: load libraries found in $FLOX_ENV_LIBS last
#   - compile the get-glibc-version program (with LIBRARY_PATH=$FLOX_ENV_LIBS)
#   - run it with no environment (using `env -i`) to observe the default
#     glibc version and confirm this does NOT match the pinned version
#   - repeat with LD_AUDIT defined and confirm that the version again does
#     not change
#   - repeat with LD_LIBRARY_PATH=$FLOX_ENV_LIBS and confirm that the
#     version does change
# 2: confirm LD_AUDIT can find missing libraries
#   - compile the get-nix-version program
#   - observe that it can run the compiled program on the sample gif
#   - unset LD_AUDIT and confirm it cannot run the program
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
  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_OLDER?}" \
    "$FLOX_BIN" init
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
#
@test "test ld-floxlib.so on Linux only" {
  if [ $(uname -s) != "Linux" ]; then
    skip "not Linux"
  fi

  # Note:
  # - installing old versions of nix (2.13.3) and glibc (2.34) for use in tests
  # - installing curl and libarchive because those packages provide libraries
  #   that are runtime dependencies of libnixmain.so
  run env _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_OLDER?}" \
    "$FLOX_BIN" install curl gcc glibc libarchive nix patchelf
  assert_success
  assert_output --partial "✅ 'curl' installed to environment"
  assert_output --partial "✅ 'gcc' installed to environment"
  assert_output --partial "✅ 'glibc' installed to environment"
  assert_output --partial "✅ 'libarchive' installed to environment"
  assert_output --partial "✅ 'nix' installed to environment"
  assert_output --partial "✅ 'patchelf' installed to environment"

  ### Test 1: load libraries found in $FLOX_ENV_LIB_DIRS last
  run "$FLOX_BIN" activate -- bash ./test-load-library-last.sh < /dev/null
  assert_success

  ### Test 2: confirm LD_AUDIT can find missing libraries
  # Link against nixmain because that's a library that won't be present on any host system.
  # Build print-nix-version, remove RUNPATH & interpreter
  run "$FLOX_BIN" activate -- bash -exc '" \
    g++ -o get-nix-version ./get-nix-version.cc -lnixmain && \
    patchelf --remove-rpath ./get-nix-version && \
    patchelf --set-interpreter "$( \
      patchelf --print-interpreter /bin/sh \
    )" ./get-nix-version && \
    LD_FLOXLIB_DEBUG=1 ./get-nix-version"'
  assert_success
  assert_output --partial "testing (Nix) 2.13.3"

  ### Test 3: confirm binary cannot find missing libraries without LD_AUDIT
  # Note run with "run -127" to silence the 127 "Command not found" error code
  # warning that bats will display by default when it attempts to launch a
  # command that fails to run because it cannot load its libraries.
  run -127 "$FLOX_BIN" activate -- bash -exc \
    '"env -i LD_DEBUG=libs ./get-nix-version"'
  assert_failure
}
