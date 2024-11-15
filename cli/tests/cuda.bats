
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
  mkdir -p "${FAKE_FHS_ROOT}/run/opengl-driver"
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
@test "cuda disabled when nvidia device absent and libcuda present" {
  run "$FLOX_BIN" activate -- bash \
    "$TESTS_DIR/cuda/cuda-disabled.sh" \
    "${FAKE_FHS_ROOT}" \
    "${TESTS_DIR}/cuda/ldconfig-mock-present.sh"
  assert_success
}

@test "cuda disabled when nvidia0 device present but libcuba absent" {
  touch "${FAKE_FHS_ROOT}/dev/nvidia0"

  run "$FLOX_BIN" activate -- bash \
    "$TESTS_DIR/cuda/cuda-disabled.sh" \
    "${FAKE_FHS_ROOT}" \
    "${TESTS_DIR}/cuda/ldconfig-mock-absent.sh"
  assert_success
}

@test "cuda disabled when nvidia0 device present but libcuba absent on NixOS" {
  touch "${FAKE_FHS_ROOT}/dev/nvidia0"

  run "$FLOX_BIN" activate -- bash \
    "$TESTS_DIR/cuda/cuda-disabled.sh" \
    "${FAKE_FHS_ROOT}" \
    "${TESTS_DIR}/cuda/ldconfig-mock-error.sh"
  assert_success
}

@test "cuda disabled when not on Linux" {
  touch "${FAKE_FHS_ROOT}/dev/nvidia0"

  run "$FLOX_BIN" activate -- bash \
    "$TESTS_DIR/cuda/cuda-disabled.sh" \
    "${FAKE_FHS_ROOT}" \
    "__LINUX_ONLY__"
  assert_success

  run "$FLOX_BIN" activate -- bash \
    "$TESTS_DIR/cuda/cuda-disabled.sh" \
    "${FAKE_FHS_ROOT}" \
    "invalid_ldconfig_path"
  assert_success
}

@test "cuda disabled when nvidia0 device present and libcuda present but manifest opts-out" {
  touch "${FAKE_FHS_ROOT}/dev/nvidia0"
  tomlq --in-place -t '.options."cuda-detection" = false' .flox/env/manifest.toml

  run "$FLOX_BIN" activate -- bash \
    "$TESTS_DIR/cuda/cuda-disabled.sh" \
    "${FAKE_FHS_ROOT}" \
    "${TESTS_DIR}/cuda/ldconfig-mock-present.sh"
  assert_success
}

@test "cuda enabled when nvidia0 device present and libcuda present" {
  touch "${FAKE_FHS_ROOT}/dev/nvidia0"

  run "$FLOX_BIN" activate -- bash \
    "$TESTS_DIR/cuda/cuda-enabled.sh" \
    "${FAKE_FHS_ROOT}" \
    "${TESTS_DIR}/cuda/ldconfig-mock-present.sh"
  assert_success
}

@test "cuda enabled when nvidia0 device present and libcuda present on NixOS" {
  touch "${FAKE_FHS_ROOT}/dev/nvidia0"
  touch "${FAKE_FHS_ROOT}/run/opengl-driver/libcuda.so"
  touch "${FAKE_FHS_ROOT}/run/opengl-driver/libcuda.so.1"
  touch "${FAKE_FHS_ROOT}/run/opengl-driver/libcudart.so"
  touch "${FAKE_FHS_ROOT}/run/opengl-driver/libcudart.so.12"
  touch "${FAKE_FHS_ROOT}/run/opengl-driver/libnvidia-ml.so"
  touch "${FAKE_FHS_ROOT}/run/opengl-driver/libnvidia-ml.so.1"
  touch "${FAKE_FHS_ROOT}/run/opengl-driver/libnvidia-nvvm.so"
  touch "${FAKE_FHS_ROOT}/run/opengl-driver/libnvidia-nvvm.so.4"

  run "$FLOX_BIN" activate -- bash \
    "$TESTS_DIR/cuda/cuda-enabled.sh" \
    "${FAKE_FHS_ROOT}" \
    "${TESTS_DIR}/cuda/ldconfig-mock-error.sh"
  assert_success
}

@test "cuda enabled when parent opts-out and nested activation doesn't" {
  touch "${FAKE_FHS_ROOT}/dev/nvidia0"

  tomlq --in-place -t '.options."cuda-detection" = false' .flox/env/manifest.toml

  NESTED_PROJECT_DIR="${PROJECT_NAME}-nested"
  "$FLOX_BIN" init -d "$NESTED_PROJECT_DIR"

  run "$FLOX_BIN" activate -d "$NESTED_PROJECT_DIR" -- bash \
    "$TESTS_DIR/cuda/cuda-enabled.sh" \
    "${FAKE_FHS_ROOT}" \
    "${TESTS_DIR}/cuda/ldconfig-mock-present.sh"
  assert_success
}

@test "cuda disabled when nested activation opts-out" {
  touch "${FAKE_FHS_ROOT}/dev/nvidia0"

  NESTED_PROJECT_DIR="${PROJECT_NAME}-nested"
  "$FLOX_BIN" init -d "$NESTED_PROJECT_DIR"
  tomlq --in-place -t '.options."cuda-detection" = false' "${NESTED_PROJECT_DIR}/.flox/env/manifest.toml"

  run "$FLOX_BIN" activate -d "$NESTED_PROJECT_DIR" -- bash \
    "$TESTS_DIR/cuda/cuda-disabled.sh" \
    "${FAKE_FHS_ROOT}" \
    "${TESTS_DIR}/cuda/ldconfig-mock-present.sh"
  assert_success
}
