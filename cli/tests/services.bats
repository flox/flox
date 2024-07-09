#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Tests for service management
#
# bats file_tags=services
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# ---------------------------------------------------------------------------- #

# Helpers for project based tests
# Note in this file, these aren't added to setup() and teardown()

project_setup() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/test"
  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" >/dev/null || return
}

project_teardown() {
  popd >/dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
}

# ---------------------------------------------------------------------------- #

setup() {
  common_test_setup
  setup_isolated_flox
  project_setup

  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.json"
}

teardown() {
  project_teardown
  common_test_teardown
}

# ---------------------------------------------------------------------------- #

@test "feature flag works" {
  RUST_LOG=flox=debug run "$FLOX_BIN" init
  refute_output --partial "service management enabled"
  unset output
  "$FLOX_BIN" delete -f
  RUST_LOG=flox=debug FLOX_FEATURES_SERVICES=true run "$FLOX_BIN" init
  assert_output --partial "service management enabled"
}

@test "can call process-compose" {
  run "$PROCESS_COMPOSE_BIN" version
  assert_success
  assert_output --partial "v1.6.1"
}

@test "process-compose can run generated config file" {
  export FLOX_FEATURES_SERVICES=true
  "$FLOX_BIN" init
  manifest_file="${TESTS_DIR}/services/touch_file/manifest.toml"
  run "$FLOX_BIN" edit -f "$manifest_file"
  assert_success
  run bash "${TESTS_DIR}/services/touch_file/check_service_ran.sh"
  assert_success
}

@test "'flox activate' with feature flag does not start services" {
  export FLOX_FEATURES_SERVICES=true
  "$FLOX_BIN" init
  manifest_file="${TESTS_DIR}/services/touch_file/manifest.toml"
  run "$FLOX_BIN" edit -f "$manifest_file"
  assert_success
  "$FLOX_BIN" activate -- true
  run [ -e hello.txt ]
  assert_failure
}

@test "'flox activate -s' starts services" {
  export FLOX_FEATURES_SERVICES=true
  "$FLOX_BIN" init
  manifest_file="${TESTS_DIR}/services/touch_file/manifest.toml"
  run "$FLOX_BIN" edit -f "$manifest_file"
  assert_success
  run bash "${TESTS_DIR}/services/touch_file/check_activation_starts_services.sh"
  assert_success
}

@test "'flox activate -s' error without feature flag" {
  export FLOX_FEATURES_SERVICES=false
  "$FLOX_BIN" init
  manifest_file="${TESTS_DIR}/services/touch_file/manifest.toml"
  run "$FLOX_BIN" edit -f "$manifest_file"
  assert_success
  unset output
  run "$FLOX_BIN" activate -s
  assert_failure
  assert_output --partial "Services are not enabled in this environment"
}
