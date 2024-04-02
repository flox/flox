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

env_setup() {
  "$FLOX_BIN" init
  "$FLOX_BIN" edit -f "$TESTS_DIR/container/manifest.toml"
}

# podman writes containers to ~/.local/share/containers/storage
# using an overlayfs.
# However, that directory is not writable
# and thus fails to be deleted by bats as part of the test teardown.
podman_cache_reset() {
  echo "Resetting podman cache" >&3
  is_linux && podman system reset --force
  true
}

# ---------------------------------------------------------------------------- #

setup() {
  common_test_setup
  setup_isolated_flox
  project_setup
  env_setup

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

  run "$FLOX_BIN" containerize
  assert_failure
  assert_output --partial "'containerize' is currently only supported on linux (found macos)."
}

# bats test_tags=containerize:default-to-file
@test "container is written to a file by default" {
  skip_if_not_linux

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
  skip_if_not_linux

  run bash -c '"$FLOX_BIN" containerize -o - | podman load'
  assert_success
  assert_line --partial "Loaded image:"
}

# bats test_tags=containerize:run-container-i
@test "container can be run with 'podman/docker run -i'" {
  skip_if_not_linux

  CONTAINER_ID="$("$FLOX_BIN" containerize -o - | podman load | sed -nr 's/^Loaded image: (.*)$/\1/p')"
  run --separate-stderr podman run -q -i "$CONTAINER_ID" true
  assert_success

  # checking
  # (1) if the variable `foo = bar` is set in the container
  # (2) if the binary `hello` is present in the container
  # (3) if the binary `hello` operates as expected
  assert_output --regexp - << EOF
bar
\/nix\/store\/.*\/bin\/hello
Hello, world!
EOF

}

# bats test_tags=containerize:run-container-no-i
@test "container can be run with 'podman/docker run'" {
  skip_if_not_linux

  CONTAINER_ID="$("$FLOX_BIN" containerize -o - | podman load | sed -nr 's/^Loaded image: (.*)$/\1/p')"
  run --separate-stderr podman run "$CONTAINER_ID" true
  assert_success

  # checking
  # (1) if the variable `foo = bar` is set in the container
  # (2) if the binary `hello` is present in the container
  # (3) if the binary `hello` operates as expected
  assert_output --regexp - << EOF
bar
\/nix\/store\/.*\/bin\/hello
Hello, world!
EOF

}

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
