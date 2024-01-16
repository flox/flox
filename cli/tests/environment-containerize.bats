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
}

# ---------------------------------------------------------------------------- #

setup() {
  common_test_setup
  project_setup
  env_setup

  mkdir -p $HOME/.config/containers
  echo '{ "default": [ {"type": "insecureAcceptAnything"} ] }' > "$HOME/.config/containers/policy.json"
}

teardown() {
  podman_cache_reset
  project_teardown
  common_test_teardown
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
  assert_output --partial "unsupported system"
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

# bats test_tags=containerize:run-container-it
@test "container can be run with 'podman/docker run -i'" {
  skip_if_not_linux

  CONTAINER_ID="$("$FLOX_BIN" containerize -o - | podman load | sed -nr 's/^Loaded image: (.*)$/\1/p')"
  run podman run -q -i "$CONTAINER_ID" true
  assert_success

  # `docker --tty` adds a carriage return to the output messing up bats assertions
  assert_equal "$(echo "${lines[0]}" | tr -d '\r')"  "bar"  # vars are present
  assert_equal "$(echo "${lines[1]#/nix/store/*/}" | tr -d '\r')"  "bin/hello"  # vars are present
  assert_equal "$(echo "${lines[2]}" | tr -d '\r')"  'Hello, world!'  # vars are present

}

# bats test_tags=containerize:run-container-no-it
@test "container can be run with 'podman/docker run'" {
  skip_if_not_linux

  CONTAINER_ID="$("$FLOX_BIN" containerize -o - | podman load | sed -nr 's/^Loaded image: (.*)$/\1/p')"
  run podman run "$CONTAINER_ID" true
  assert_success
  assert_line --index 0 "bar"                   # vars are present
  assert_line --index 1 --regexp ".*/bin/hello" # programs are present
  assert_line --index 2 'Hello, world!'         # programs execute
}

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
