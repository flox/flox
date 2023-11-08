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
  run "$FLOX_CLI" activate -d "$PROJECT_DIR";
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


# ---------------------------------------------------------------------------- #

@test "'build-env' builds fresh environment" {
  skip "FIXME: needs 'url' field"
  run "$FLOX_CLI" init;
  assert_success;
  run "$BUILD_ENV_BIN" "$NIX_BIN" \
    "$NIX_SYSTEM" \
    "$PROJECT_DIR/.flox/env/manifest.lock" \
    "$PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME" \
    "$ENV_FROM_LOCKFILE_PATH";
  assert_success;
  run [ -d "$PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME" ];
  assert_success;
}
