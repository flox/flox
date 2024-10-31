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
  pushd "$PROJECT_DIR" > /dev/null || return
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.json"
}

project_teardown() {
  popd > /dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
}

# ---------------------------------------------------------------------------- #

setup() {
  common_test_setup
  setup_isolated_flox
  project_setup
}

teardown() {
  wait_for_watchdogs "$PROJECT_DIR"
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
✨ Created environment 'test' ($NIX_SYSTEM)

Next:
  $ flox search <package>    <- Search for a package
  $ flox install <package>   <- Install a package into an environment
  $ flox activate            <- Enter the environment
  $ flox edit                <- Add environment variables and shell hooks
EOF

}

@test "c8: names don't conflict with floxhub: when naming with flox init -e do not allow '/'" {
  run "$FLOX_BIN" init -n "owner/name"
  assert_failure
}

@test "c8: names don't conflict with floxhub: when naming with flox init -e do not allow ' ' (space)" {
  run "$FLOX_BIN" init -n "na me"
  assert_failure
}

function check_with_dir() {
  run ls -A "$PROJECT_DIR"
  assert_output "other"
  run ls -A "$PROJECT_DIR/other"
  assert_output ".flox"
}

@test "c2.1: \`flox init\` with \`--dir .\` will create an environment in current working directory." {
  run "$FLOX_BIN" init -d .
  assert_success
  run ls -A
  assert_output ".flox"
}

@test "c2.1: \`flox init\` with \`--dir ..\` will create an environment in parent working directory." {
  mkdir -p "$PROJECT_DIR/other"

  pushd other
  run "$FLOX_BIN" init -d ..
  assert_success
  popd

  run ls -A
  assert_output - <<EOF
.flox
other
EOF
}

@test "c2.1: \`flox init\` with \`--dir <path>\` will create an environment in \`<path>\`. (relative)" {
  mkdir -p "$PROJECT_DIR/other"

  run "$FLOX_BIN" init -d ./other
  assert_success
  check_with_dir
}

@test "c2.1: \`flox init\` with \`--dir <path>\` will not create an environment where \`<path>\` is a file" {
  touch "$PROJECT_DIR/other"

  run "$FLOX_BIN" init -d ./other
  assert_failure
  assert_line --partial "Could not prepare a '.flox' directory: Not a directory"
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

# bats test_tags=init:gitignore
@test "c9: flox init adds .gitingore that ignores run/ directory" {
  "$FLOX_BIN" init
  run cat .flox/.gitignore
  assert_success
  assert_line "run/"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=init:python:requirements
@test "'flox init' sets up a working Python environment that works across all methods of activate" {
  OWNER="owner"
  NAME="name"

  echo "requests" > requirements.txt

  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/init/python_requests.json"
  "$FLOX_BIN" init --auto-setup --name "$NAME"
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.json"

  FLOX_SHELL=bash "$FLOX_BIN" activate -- python -c "import requests"
  FLOX_SHELL=zsh "$FLOX_BIN" activate -- python -c "import requests"

  floxhub_setup "$OWNER"

  "$FLOX_BIN" push --owner "$OWNER"

  "$FLOX_BIN" delete -f

  "$FLOX_BIN" pull "$OWNER/$NAME"

  FLOX_SHELL=bash "$FLOX_BIN" activate -- python -c "import requests"
  FLOX_SHELL=zsh "$FLOX_BIN" activate -- python -c "import requests"

  "$FLOX_BIN" delete -f

  FLOX_SHELL=bash "$FLOX_BIN" activate --trust -r "$OWNER/$NAME" -- python -c "import requests"
  FLOX_SHELL=zsh "$FLOX_BIN" activate --trust -r "$OWNER/$NAME" -- python -c "import requests"
}

# bats test_tags=init:catalog
@test "init creates manifest with all 4 systems" {
  "$FLOX_BIN" init
  systems=$(tomlq -r -c '.options.systems' .flox/env/manifest.toml)
  assert_equal "$systems" '["aarch64-darwin","aarch64-linux","x86_64-darwin","x86_64-linux"]'
}

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
