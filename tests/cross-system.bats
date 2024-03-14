
#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test if cross system push/pull works.
#
# This is a very similar test to that in environment-pull.bats,
# but it tests across multiple machines.
# To use multiple machines, we rely on the environment pushed from a different
# system in the previous CI run.
# It uses credentials stored in a GH action secret to authenticate with the
# development floxhub instance.
#
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# bats file_tags=end2end

# ---------------------------------------------------------------------------- #

setup_file() {
  common_file_setup
  "$FLOX_BIN" config --set floxhub_url "https://hub.preview.flox.dev/"
  if [ -z "${FLOXEM_FLOXTEST_TOKEN:-}" ]; then
    skip "FLOXEM_FLOXTEST_TOKEN is not set"
  fi
  export FLOX_FLOXHUB_TOKEN="$FLOXEM_FLOXTEST_TOKEN"
  export OWNER="floxEM"
}

teardown_file() {
  "$FLOX_BIN" config --delete floxhub_url
  unset FLOX_FLOXHUB_TOKEN
}

# ---------------------------------------------------------------------------- #

# Helpers for project based tests.

project_setup() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/project-${BATS_TEST_NUMBER?}"
  export PROJECT_NAME="${PROJECT_DIR##*/}"
  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" > /dev/null || return
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

@test "flox push succeeds" {
  name="created-on-$NIX_SYSTEM"

  "$FLOX_BIN" init -n "$name"
  "$FLOX_BIN" install hello
  run "$FLOX_BIN" push --owner "$OWNER" --force
  assert_success
  assert_output --partial "pushed to FloxHub (forced)"
}

# This should pull the environment created by the previous run on a different
# system of the flox push test above.
# Because we don't check for anything other than hello being installed,
# hopefully race conditions won't be an issue.
@test "can flox pull and activate an environment created on another system" {
  local pull_system
  case "$NIX_SYSTEM" in
    x86_64-linux)
      pull_system="x86_64-darwin"
      ;;
    x86_64-darwin)
      pull_system="x86_64-linux"
      ;;
    *)
      # we only run the above two systems consistently in CI
      skip "unsupported system: $NIX_SYSTEM"
      ;;
  esac

  name="created-on-$pull_system"

  "$FLOX_BIN" pull "$OWNER/$name" --force
  # Close fd 3 because of
  # https://bats-core.readthedocs.io/en/stable/writing-tests.html#file-descriptor-3-read-this-if-bats-hangs
  run "$FLOX_BIN" activate -- hello 3>&-
  assert_success
  assert_output --partial "Hello"
}
