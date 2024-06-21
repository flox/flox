#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test flox conteinerize
#
# bats file_tags=containerize
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# ---------------------------------------------------------------------------- #

# Helpers for project based tests.

project_setup() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/test"
  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" > /dev/null || return
}

project_teardown() {
  popd > /dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
}

env_setup_catalog() {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json"
  "$FLOX_BIN" init
  "$FLOX_BIN" edit -f "$TESTS_DIR/container/manifest1.toml"
}

env_setup() {
  "$FLOX_BIN" init
  "$FLOX_BIN" edit -f "$TESTS_DIR/container/manifest0.toml"
}

# podman writes containers to ~/.local/share/containers/storage
# using an overlayfs.
# However, that directory is not writable
# and thus fails to be deleted by bats as part of the test teardown.
podman_cache_reset() {
  # echo "Resetting podman cache" >&3
  is_linux && podman system reset --force
  true
}

# ---------------------------------------------------------------------------- #

setup() {
  common_test_setup
  setup_isolated_flox
  project_setup

  mkdir -p $HOME/.config/containers
  echo '{ "default": [ {"type": "insecureAcceptAnything"} ] }' > "$HOME/.config/containers/policy.json"
}

teardown() {
  project_teardown
  common_test_teardown
}

teardown_file() {
  podman_cache_reset
}

# ---------------------------------------------------------------------------- #

function is_linux() {
  [[ "$(uname)" == "Linux" ]]
}

function skip_if_not_linux() {
  if ! is_linux; then
    skip "Only available on linux"
  fi
}

function skip_if_linux() {
  if is_linux; then
    skip "Not applicable on linux"
  fi
}

# ---------------------------------------------------------------------------- #

# bats test_tags=containerize:unsupported
@test "building a container fails on macos" {

  skip_if_linux

  "$FLOX_BIN" init

  run "$FLOX_BIN" containerize
  assert_failure
  assert_output --partial "'containerize' is currently only supported on linux (found macos)."
}

# bats test_tags=containerize:default-to-file
@test "container is written to a file by default" {
  export FLOX_FEATURES_USE_CATALOG=false
  skip_if_not_linux

  env_setup

  run "$FLOX_BIN" containerize
  assert_success

  assert [ -f "test-container.tar.gz" ] # <env-name>-container.tar.gz by default

  run which podman

  run podman load -i test-container.tar.gz
  assert_success
  assert_line --partial "Loaded image:"
}

# bats test_tags=containerize:default-to-file
@test "catalog: container is written to a file by default" {
  skip_if_not_linux

  env_setup_catalog

  run "$FLOX_BIN" containerize
  assert_success

  assert [ -f "test-container.tar.gz" ] # <env-name>-container.tar.gz by default

  run which podman

  run podman load -i test-container.tar.gz
  assert_success
  assert_line --partial "Loaded image:"
}

# bats test_tags=containerize:piped-to-stdout
@test "container is written to stdout when '-o -' is passed" {
  export FLOX_FEATURES_USE_CATALOG=false
  skip_if_not_linux

  env_setup

  run bash -c '"$FLOX_BIN" containerize -o - | podman load'
  assert_success
  assert_line --partial "Loaded image:"
}

# bats test_tags=containerize:piped-to-stdout
@test "catalog: container is written to stdout when '-o -' is passed" {
  skip_if_not_linux

  env_setup_catalog

  run bash -c '"$FLOX_BIN" containerize -o - | podman load'
  assert_success
  assert_line --partial "Loaded image:"
}

# bats test_tags=containerize:run-container-i
@test "container can be run with 'podman/docker run -i'" {
  export FLOX_FEATURES_USE_CATALOG=false
  skip_if_not_linux

  env_setup

  CONTAINER_ID="$("$FLOX_BIN" containerize -o - | podman load | sed -nr 's/^Loaded image: (.*)$/\1/p')"
  run --separate-stderr podman run -q -i "$CONTAINER_ID" -c 'echo $foo'
  assert_success

  # check:
  # (1) if the variable `foo = bar` is set in the container
  #   - printed to STDOUT by the container invocation
  # (2) if the binary `hello` is present in the container
  # (3) if the binary `hello` operates as expected
  #   - printed to STDOUT by the on-activate hook, but then
  #     redirected to STDERR by the flox activate script
  assert_equal "${#lines[@]}" 1 # 1 result
  assert_equal "${lines[0]}" "bar"

  # Podman generates some errors/warnings about UIDs/GIDs due to how the rootless
  # setup works: https://github.com/containers/podman/issues/15611
  # Another error you may see is that the container file already exists, which is
  # harmless and can be ignored.
  # So, we can't rely on the *number* of stderr lines, but we know the lines we
  # care about will be the last two lines.

  n_stderr_lines="${#stderr_lines[@]}"
  hello_line="$(($n_stderr_lines - 1))"
  store_path_line="$(($n_stderr_lines - 2))"
  assert_regex "${stderr_lines[$store_path_line]}" "\/nix\/store\/.*\/bin\/hello"
  assert_equal "${stderr_lines[$hello_line]}" "Hello, world!"
}

# bats test_tags=containerize:run-container-i
@test "catalog: container can be run with 'podman/docker run -i'" {
  skip_if_not_linux

  env_setup_catalog

  CONTAINER_ID="$("$FLOX_BIN" containerize -o - | podman load | sed -nr 's/^Loaded image: (.*)$/\1/p')"
  run --separate-stderr podman run -q -i "$CONTAINER_ID" -c 'echo $foo'
  assert_success

  # check:
  # (1) if the variable `foo = bar` is set in the container
  #   - printed to STDOUT by the container invocation
  # (2) if the binary `hello` is present in the container
  # (3) if the binary `hello` operates as expected
  #   - printed to STDOUT by the on-activate hook, but then
  #     redirected to STDERR by the flox activate script
  assert_equal "${#lines[@]}" 1 # 1 result
  assert_equal "${lines[0]}" "bar"

  # Podman generates some errors/warnings about UIDs/GIDs due to how the rootless
  # setup works: https://github.com/containers/podman/issues/15611
  # Another error you may see is that the container file already exists, which is
  # harmless and can be ignored.
  # So, we can't rely on the *number* of stderr lines, but we know the lines we
  # care about will be the last two lines.

  n_stderr_lines="${#stderr_lines[@]}"
  hello_line="$(($n_stderr_lines - 1))"
  store_path_line="$(($n_stderr_lines - 2))"
  assert_regex "${stderr_lines[$store_path_line]}" "\/nix\/store\/.*\/bin\/hello"
  assert_equal "${stderr_lines[$hello_line]}" "Hello, world!"
}

# bats test_tags=containerize:run-container-no-i
@test "container can be run with 'podman/docker run'" {
  skip_if_not_linux

  export FLOX_FEATURES_USE_CATALOG=false
  env_setup

  CONTAINER_ID="$("$FLOX_BIN" containerize -o - | podman load | sed -nr 's/^Loaded image: (.*)$/\1/p')"
  run --separate-stderr podman run "$CONTAINER_ID" -c 'echo $foo'
  assert_success

  assert_equal "${#lines[@]}" 1 # 1 result
  assert_equal "${lines[0]}" "bar"

  # Podman generates some errors/warnings about UIDs/GIDs due to how the rootless
  # setup works: https://github.com/containers/podman/issues/15611
  # Another error you may see is that the container file already exists, which is
  # harmless and can be ignored.
  # So, we can't rely on the *number* of stderr lines, but we know the lines we
  # care about will be the last two lines.

  n_stderr_lines="${#stderr_lines[@]}"
  hello_line="$(($n_stderr_lines - 1))"
  store_path_line="$(($n_stderr_lines - 2))"
  assert_regex "${stderr_lines[$store_path_line]}" "\/nix\/store\/.*\/bin\/hello"
  assert_equal "${stderr_lines[$hello_line]}" "Hello, world!"
}

# bats test_tags=containerize:run-container-no-i
@test "catalog: container can be run with 'podman/docker run'" {
  skip_if_not_linux

  env_setup_catalog

  CONTAINER_ID="$("$FLOX_BIN" containerize -o - | podman load | sed -nr 's/^Loaded image: (.*)$/\1/p')"
  run --separate-stderr podman run "$CONTAINER_ID" -c 'echo $foo'
  assert_success

  assert_equal "${#lines[@]}" 1 # 1 result
  assert_equal "${lines[0]}" "bar"

  # Podman generates some errors/warnings about UIDs/GIDs due to how the rootless
  # setup works: https://github.com/containers/podman/issues/15611
  # Another error you may see is that the container file already exists, which is
  # harmless and can be ignored.
  # So, we can't rely on the *number* of stderr lines, but we know the lines we
  # care about will be the last two lines.

  n_stderr_lines="${#stderr_lines[@]}"
  hello_line="$(($n_stderr_lines - 1))"
  store_path_line="$(($n_stderr_lines - 2))"
  assert_regex "${stderr_lines[$store_path_line]}" "\/nix\/store\/.*\/bin\/hello"
  assert_equal "${stderr_lines[$hello_line]}" "Hello, world!"
}

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
