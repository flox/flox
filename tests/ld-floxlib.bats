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
#   - compile the print-gif-info program
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
  cp ./harnesses/ld-floxlib/* "$PROJECT_DIR"
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

  run env _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_OLDER?}" \
    "$FLOX_BIN" install gcc glibc giflib patchelf
  assert_success
  assert_output --partial "✅ 'gcc' installed to environment"
  assert_output --partial "✅ 'glibc' installed to environment"
  assert_output --partial "✅ 'giflib' installed to environment"
  assert_output --partial "✅ 'patchelf' installed to environment"

  ### Test 1: load libraries found in $FLOX_ENV_LIB_DIRS last
  run "$FLOX_BIN" activate -- bash ./test-load-library-last.sh < /dev/null
  assert_success

  ### Test 2: confirm LD_AUDIT can find missing libraries
  run "$FLOX_BIN" activate -- bash -exc \
    '"cc -o pgi ./print-gif-info.c -lgif && ./pgi ./flox-edge.gif"'
  assert_output --partial "GIF Information for: ./flox-edge.gif"
  assert_output --partial "Number of frames: 0"
  assert_output --partial "Width: 270 pixels"
  assert_output --partial "Height: 137 pixels"
}
