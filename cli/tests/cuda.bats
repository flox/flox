
#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test CUDA detection during `flox activate`.
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# bats file_tags=cuda

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
  pushd "$PROJECT_DIR" > /dev/null || return
  "$FLOX_BIN" init -d "$PROJECT_DIR"

  export FAKE_FHS_ROOT="${PROJECT_DIR}/fake_fhs_root"
  mkdir "$FAKE_FHS_ROOT"
  mkdir -p "${FAKE_FHS_ROOT}/dev"
}

project_teardown() {
  popd > /dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
  unset PROJECT_NAME
  unset FAKE_FHS_ROOT
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
@test "cuda disabled when nvidia device absent" {
  FLOX_SHELL=bash run "$FLOX_BIN" activate -- bash "$TESTS_DIR/cuda/cuda-disabled.sh" "${FAKE_FHS_ROOT}"
  assert_success
}

@test "cuda disabled when nvidia0 device present but libcuba absent" {
  touch "${FAKE_FHS_ROOT}/dev/nvidia0"

  FLOX_SHELL=bash run "$FLOX_BIN" activate -- bash "$TESTS_DIR/cuda/cuda-disabled.sh" "${FAKE_FHS_ROOT}"
  assert_success
}

@test "cuda disabled when nvidia0 device present and libcuda present but manifest opts-out" {
  touch "${FAKE_FHS_ROOT}/dev/nvidia0"
  mkdir -p "${FAKE_FHS_ROOT}/run/opengl-drivers"
  touch "${FAKE_FHS_ROOT}/run/opengl-drivers/libcuda.so.1"
  tomlq --in-place -t '.options."cuda-detection" = false' .flox/env/manifest.toml

  FLOX_SHELL=bash run "$FLOX_BIN" activate -- bash "$TESTS_DIR/cuda/cuda-disabled.sh" "${FAKE_FHS_ROOT}"
  assert_success
}

@test "cuda enabled when nvidia0 device present and libcuda present" {
  touch "${FAKE_FHS_ROOT}/dev/nvidia0"
  mkdir -p "${FAKE_FHS_ROOT}/run/opengl-drivers"
  touch "${FAKE_FHS_ROOT}/run/opengl-drivers/libcuda.so.1"

  FLOX_SHELL=bash run "$FLOX_BIN" activate -- bash "$TESTS_DIR/cuda/cuda-enabled.sh" "${FAKE_FHS_ROOT}"
  assert_success
  assert_output --partial "${PROJECT_DIR}/.flox/lib/libcuda.so.1"
}

@test "cuda enabled when nvidia1 device present and multiple libraries present in alternate directory" {
  touch "${FAKE_FHS_ROOT}/dev/nvidia1"
  mkdir -p "${FAKE_FHS_ROOT}/usr/local/lib"
  touch "${FAKE_FHS_ROOT}/usr/local/lib/libcuda.so.1"
  touch "${FAKE_FHS_ROOT}/usr/local/lib/libnvidia.so.1"
  touch "${FAKE_FHS_ROOT}/usr/local/lib/libdxcore.so.1"

  FLOX_SHELL=bash run "$FLOX_BIN" activate -- bash "$TESTS_DIR/cuda/cuda-enabled.sh" "${FAKE_FHS_ROOT}"
  assert_success
  assert_output --partial "${PROJECT_DIR}/.flox/lib/libcuda.so.1"
  assert_output --partial "${PROJECT_DIR}/.flox/lib/libnvidia.so.1"
  assert_output --partial "${PROJECT_DIR}/.flox/lib/libdxcore.so.1"
}

@test "cuda enabled when nested activation doesn't opt-out" {
  touch "${FAKE_FHS_ROOT}/dev/nvidia0"
  mkdir -p "${FAKE_FHS_ROOT}/run/opengl-drivers"
  touch "${FAKE_FHS_ROOT}/run/opengl-drivers/libcuda.so.1"

  tomlq --in-place -t '.options."cuda-detection" = false' .flox/env/manifest.toml

  NESTED_PROJECT_DIR="${PROJECT_NAME}-nested"
  "$FLOX_BIN" init -d "$NESTED_PROJECT_DIR"

  FLOX_SHELL=bash run "$FLOX_BIN" activate -- \
    "$FLOX_BIN" activate -d "$NESTED_PROJECT_DIR" -- \
    bash "$TESTS_DIR/cuda/cuda-enabled.sh" "${FAKE_FHS_ROOT}"
  assert_success
}

# This is the current, rather than necessarily desired, behaviour.
@test "cuda enabled when nested activation attempts to opt-out" {
  touch "${FAKE_FHS_ROOT}/dev/nvidia0"
  mkdir -p "${FAKE_FHS_ROOT}/run/opengl-drivers"
  touch "${FAKE_FHS_ROOT}/run/opengl-drivers/libcuda.so.1"

  NESTED_PROJECT_DIR="${PROJECT_NAME}-nested"
  "$FLOX_BIN" init -d "$NESTED_PROJECT_DIR"
  tomlq --in-place -t '.options."cuda-detection" = false' "${NESTED_PROJECT_DIR}/.flox/env/manifest.toml"

  FLOX_SHELL=bash run "$FLOX_BIN" activate -- \
    "$FLOX_BIN" activate -d "$NESTED_PROJECT_DIR" -- \
    bash "$TESTS_DIR/cuda/cuda-enabled.sh" "${FAKE_FHS_ROOT}"
  assert_success
}
