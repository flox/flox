#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test that we can build environments
#
#
# ---------------------------------------------------------------------------- #

load test_support.bash;

# bats file_tags=activate,init


# ---------------------------------------------------------------------------- #

setup_file() {
  common_file_setup;
}


# ---------------------------------------------------------------------------- #

# Helpers for project based tests.

project_setup() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/project-${BATS_TEST_NUMBER?}";
  export PROJECT_NAME="${PROJECT_DIR##*/}";
  rm -rf "$PROJECT_DIR";
  mkdir -p "$PROJECT_DIR";
  pushd "$PROJECT_DIR" >/dev/null||return;
  git init;
}

project_teardown() {
  popd >/dev/null||return;
  rm -rf "${PROJECT_DIR?}";
  unset PROJECT_DIR;
  unset PROJECT_NAME;
}

activate_local_env() {
  run "$FLOX_BIN" activate -d "$PROJECT_DIR";
}


# ---------------------------------------------------------------------------- #

setup() {
  common_test_setup;
  project_setup;
}

teardown() {
  project_teardown;
  common_test_teardown;
}
