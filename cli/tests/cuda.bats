
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

# Non-exhaustive selection of patterns from the mocked ldconfig output and
# the NixOS fixture directory. NB: libdxcore isn't covered by the mock.
assert_cuda_libs_present() {
  local lib_path="$1"
  if [ -z "$lib_path" ]; then
    echo "LD_FLOXLIB_FILES_PATH was not modified and it should have been"
    return 1
  fi
  echo "LD_FLOXLIB_FILES_PATH=$lib_path" >&2
  declare -a expected=(
    "libcuda.so"
    "libcuda.so.1"
    "libcudart.so"
    "libcudart.so.12"
    "libnvvm.so"
    "libnvvm.so.4"
    "libnvrtc.so"
    "libnvrtc.so.12"
    "libnvidia-ml.so"
    "libnvidia-ml.so.1"
    "libnvidia-nvvm.so"
    "libnvidia-nvvm.so.4"
  )
  local IFS=":"
  for pattern in "${expected[@]}"; do
    echo "Checking for ${pattern}" 1>&2
    echo $lib_path \
      | xargs -n 1 basename | grep "^${pattern}$" > /dev/null \
      || { echo "Failed to find ${pattern}" 1>&2; return 1; }
  done
}

# ---------------------------------------------------------------------------- #
#
@test "cuda disabled when nvidia device absent and libcuda present" {
  run env -u LD_FLOXLIB_FILES_PATH \
    _FLOX_TESTING_CUDA_FHS_ROOT="${FAKE_FHS_ROOT}" \
    _FLOX_TESTING_CUDA_LDCONFIG="${TESTS_DIR}/cuda/ldconfig-mock-present.sh" \
    "$FLOX_BIN" activate -- printenv LD_FLOXLIB_FILES_PATH
  [ -z "$output" ]
}

@test "cuda disabled when nvidia0 device present but libcuba absent" {
  touch "${FAKE_FHS_ROOT}/dev/nvidia0"

  run env -u LD_FLOXLIB_FILES_PATH \
    _FLOX_TESTING_CUDA_FHS_ROOT="${FAKE_FHS_ROOT}" \
    _FLOX_TESTING_CUDA_LDCONFIG="${TESTS_DIR}/cuda/ldconfig-mock-absent.sh" \
    "$FLOX_BIN" activate -- printenv LD_FLOXLIB_FILES_PATH
  [ -z "$output" ]
}

@test "cuda disabled when nvidia0 device present but libcuba absent on NixOS" {
  touch "${FAKE_FHS_ROOT}/dev/nvidia0"
  mkdir -p "${FAKE_FHS_ROOT}/run/opengl-driver"

  run env -u LD_FLOXLIB_FILES_PATH \
    _FLOX_TESTING_CUDA_FHS_ROOT="${FAKE_FHS_ROOT}" \
    _FLOX_TESTING_CUDA_LDCONFIG="${TESTS_DIR}/cuda/ldconfig-mock-error.sh" \
    "$FLOX_BIN" activate -- printenv LD_FLOXLIB_FILES_PATH
  [ -z "$output" ]
}

@test "cuda disabled when not on Linux" {
  touch "${FAKE_FHS_ROOT}/dev/nvidia0"

  run env -u LD_FLOXLIB_FILES_PATH \
    _FLOX_TESTING_CUDA_FHS_ROOT="${FAKE_FHS_ROOT}" \
    _FLOX_TESTING_CUDA_LDCONFIG="__LINUX_ONLY__" \
    "$FLOX_BIN" activate -- printenv LD_FLOXLIB_FILES_PATH
  [ -z "$output" ]

  run env -u LD_FLOXLIB_FILES_PATH \
    _FLOX_TESTING_CUDA_FHS_ROOT="${FAKE_FHS_ROOT}" \
    _FLOX_TESTING_CUDA_LDCONFIG="invalid_ldconfig_path" \
    "$FLOX_BIN" activate -- printenv LD_FLOXLIB_FILES_PATH
  [ -z "$output" ]
}

@test "cuda disabled when nvidia0 device present and libcuda present but manifest opts-out" {
  touch "${FAKE_FHS_ROOT}/dev/nvidia0"
  tomlq --in-place -t '.options."cuda-detection" = false' .flox/env/manifest.toml

  run env -u LD_FLOXLIB_FILES_PATH \
    _FLOX_TESTING_CUDA_FHS_ROOT="${FAKE_FHS_ROOT}" \
    _FLOX_TESTING_CUDA_LDCONFIG="${TESTS_DIR}/cuda/ldconfig-mock-present.sh" \
    "$FLOX_BIN" activate -- printenv LD_FLOXLIB_FILES_PATH
  [ -z "$output" ]
}

@test "cuda enabled when nvidia0 device present and libcuda present" {
  touch "${FAKE_FHS_ROOT}/dev/nvidia0"

  run env -u LD_FLOXLIB_FILES_PATH \
    _FLOX_TESTING_CUDA_FHS_ROOT="${FAKE_FHS_ROOT}" \
    _FLOX_TESTING_CUDA_LDCONFIG="${TESTS_DIR}/cuda/ldconfig-mock-present.sh" \
    "$FLOX_BIN" activate -- printenv LD_FLOXLIB_FILES_PATH
  assert_success
  assert_cuda_libs_present "$output"
}

@test "cuda enabled when nvidia0 device present and libcuda present on NixOS" {
  touch "${FAKE_FHS_ROOT}/dev/nvidia0"
  mkdir -p "${FAKE_FHS_ROOT}/run/opengl-driver"
  touch "${FAKE_FHS_ROOT}/run/opengl-driver/libcuda.so"
  touch "${FAKE_FHS_ROOT}/run/opengl-driver/libcuda.so.1"
  touch "${FAKE_FHS_ROOT}/run/opengl-driver/libcudart.so"
  touch "${FAKE_FHS_ROOT}/run/opengl-driver/libcudart.so.12"
  touch "${FAKE_FHS_ROOT}/run/opengl-driver/libnvvm.so"
  touch "${FAKE_FHS_ROOT}/run/opengl-driver/libnvvm.so.4"
  touch "${FAKE_FHS_ROOT}/run/opengl-driver/libnvrtc.so"
  touch "${FAKE_FHS_ROOT}/run/opengl-driver/libnvrtc.so.12"
  touch "${FAKE_FHS_ROOT}/run/opengl-driver/libnvidia-ml.so"
  touch "${FAKE_FHS_ROOT}/run/opengl-driver/libnvidia-ml.so.1"
  touch "${FAKE_FHS_ROOT}/run/opengl-driver/libnvidia-nvvm.so"
  touch "${FAKE_FHS_ROOT}/run/opengl-driver/libnvidia-nvvm.so.4"

  run env -u LD_FLOXLIB_FILES_PATH \
    _FLOX_TESTING_CUDA_FHS_ROOT="${FAKE_FHS_ROOT}" \
    _FLOX_TESTING_CUDA_LDCONFIG="${TESTS_DIR}/cuda/ldconfig-mock-error.sh" \
    "$FLOX_BIN" activate -- printenv LD_FLOXLIB_FILES_PATH
  assert_success
  assert_cuda_libs_present "$output"
}

@test "cuda enabled when parent opts-out and nested activation doesn't" {
  touch "${FAKE_FHS_ROOT}/dev/nvidia0"

  tomlq --in-place -t '.options."cuda-detection" = false' .flox/env/manifest.toml

  NESTED_PROJECT_DIR="${PROJECT_NAME}-nested"
  "$FLOX_BIN" init -d "$NESTED_PROJECT_DIR"

  run env -u LD_FLOXLIB_FILES_PATH \
    _FLOX_TESTING_CUDA_FHS_ROOT="${FAKE_FHS_ROOT}" \
    _FLOX_TESTING_CUDA_LDCONFIG="${TESTS_DIR}/cuda/ldconfig-mock-present.sh" \
    "$FLOX_BIN" activate -d "$NESTED_PROJECT_DIR" -- printenv LD_FLOXLIB_FILES_PATH
  assert_success
  assert_cuda_libs_present "$output"
}

@test "cuda disabled when nested activation opts-out" {
  touch "${FAKE_FHS_ROOT}/dev/nvidia0"

  NESTED_PROJECT_DIR="${PROJECT_NAME}-nested"
  "$FLOX_BIN" init -d "$NESTED_PROJECT_DIR"
  tomlq --in-place -t '.options."cuda-detection" = false' "${NESTED_PROJECT_DIR}/.flox/env/manifest.toml"

  run env -u LD_FLOXLIB_FILES_PATH \
    _FLOX_TESTING_CUDA_FHS_ROOT="${FAKE_FHS_ROOT}" \
    _FLOX_TESTING_CUDA_LDCONFIG="${TESTS_DIR}/cuda/ldconfig-mock-present.sh" \
    "$FLOX_BIN" activate -d "$NESTED_PROJECT_DIR" -- printenv LD_FLOXLIB_FILES_PATH
  [ -z "$output" ]
}
