#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test that we can have an authentication flow
#
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# bats file_tags=auth

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
  unset FLOX_FLOXHUB_TOKEN
  "$FLOX_BIN" auth logout
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

# bats test_tags=auth,auth:login:notty
@test "auth login fails if not a tty" {
  run "$FLOX_BIN" auth login
  assert_failure
}

# ---------------------------------------------------------------------------- #

# bats test_tags=auth,auth:login:messages
@test "'auth login' asks to press [enter]" {
  # fixup linux ci systems:
  # * ensure we simulate having a display
  # * ensure we simulate having an opener
  export DISPLAY="1"
  mkdir -p ./bin
  touch ./bin/xdg-open
  export PATH="$PWD/bin:$PATH"

  run expect "$TESTS_DIR/auth/loginPrompt.exp"
  assert_success
  assert_line --partial "First copy your one-time code:"
  assert_line --regexp "Press enter to open .+ in your browser\.\.\."
}

# ---------------------------------------------------------------------------- #

# bats test_tags=auth,auth:login:in-ssh
@test "'auth login' detects we are in an ssh session" {
  # fixup linux ci systems:
  # * ensure we simulate having a display
  # * ensure we simulate having an opener
  export DISPLAY="1"
  mkdir -p ./bin
  touch ./bin/xdg-open
  export PATH="$PWD/bin:$PATH"

  # emulate ssh environment
  export SSH_TTY="1"

  run expect "$TESTS_DIR/auth/loginPrompt.exp"
  assert_success
  assert_line --regexp "Go to .+ in your browser"
  assert_line --partial "Then enter your one-time code: "
}
