#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test flox init
#
# bats file_tags=init
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# ---------------------------------------------------------------------------- #

# Helpers for project based tests.

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
  project_setup
}

teardown() {
  project_teardown
  common_test_teardown
}


# ---------------------------------------------------------------------------- #

@test "c2: flox init without a name should create an environment named the same as the directory the user is in" {

  run "$FLOX_BIN" init
  assert_success

  run cat .flox/env.json
  assert_success
  assert_output --partial '"name": "test"'
}

@test "c2: If the user is in ~ the environment should be called 'default'." {

  skip "Can't mock user / home dir"

  export HOME="$PROJECT_DIR"

  run "$FLOX_BIN" init
  assert_success

  run cat .flox/env.json
  assert_success
  assert_output --partial '"name": "default"'

}

@test "c4: custom name option 1: flox init accepts -n for a user defined name" {
  run "$FLOX_BIN" init -n "other-test"
  assert_success

  run cat .flox/env.json
  assert_success
  assert_output --partial '"name": "other-test"'
}

@test "c4: custom name option 1: flox init accepts --name for a user defined name" {
  run "$FLOX_BIN" init --name "other-test"
  assert_success

  run cat .flox/env.json
  assert_success
  assert_output --partial '"name": "other-test"'
}

@test "c6: a single directory for state" {
  run "$FLOX_BIN" init
  assert_success

  run ls -A
  assert_output ".flox"
}

@test "c7: confirmation with tips" {
  run "$FLOX_BIN" init
  assert_success

  assert_output - <<EOF
✨ Created environment test ($NIX_SYSTEM)

Next:
  $ flox search <package>    <- Search for a package
  $ flox install <package>   <- Install a package into an environment
  $ flox activate            <- Enter the environment
EOF

}

@test "c8: names don't conflict with flox hub: when naming with flox init -e do not allow '/'" {
  run "$FLOX_BIN" init -n "owner/name"
  assert_failure
}

@test "c8: names don't conflict with flox hub: when naming with flox init -e do not allow ' ' (space)" {
  run "$FLOX_BIN" init -n "na me"
  assert_failure
}


function check_with_dir() {
  run ls -A "$PROJECT_DIR"
  assert_output "other"
  run ls -A "$PROJECT_DIR/other"
  assert_output ".flox"
}

@test "c2.1: \`flox init\` with \`--dir <path>\` will create an environment in \`<path>\`. (relative)" {
  mkdir -p "$PROJECT_DIR/other"

  run "$FLOX_BIN" init -d ./other
  assert_success
  check_with_dir
}


@test "c2.1: \`flox init\` with \`--dir <path>\` will create an environment in \`<path>\`. (absolute)" {
  mkdir -p "$PROJECT_DIR/other"

  run "$FLOX_BIN" init -d "$PROJECT_DIR/other"
  assert_success
  check_with_dir
}

@test "c2.1: \`flox init\` with \`--dir <path>\` will create an environment in \`<path>\`. (create dir)" {
  run "$FLOX_BIN" init -d "$PROJECT_DIR/other"
  assert_success
  check_with_dir
}

# bats test_tags=init,init:gitignore
@test "c9: flox init adds .gitingore that ignores run/ directory" {
  "$FLOX_BIN" init
  run cat .flox/.gitignore
  assert_success
  assert_line "run/"
}

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
