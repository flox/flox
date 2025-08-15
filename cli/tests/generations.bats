#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test rust impl of `flox generations`
#
# ---------------------------------------------------------------------------- #

load test_support.bash
# bats file_tags=generations

# ---------------------------------------------------------------------------- #

# Helpers for project based tests.

project_setup() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/project-${BATS_TEST_NUMBER?}"
  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" >/dev/null || return
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.yaml"
}

project_teardown() {
  popd >/dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
}

# ---------------------------------------------------------------------------- #

setup() {
  common_test_setup
  setup_isolated_flox
  project_setup
  floxhub_setup "owner"
}
teardown() {
  project_teardown
  common_test_teardown
}

# ---------------------------------------------------------------------------- #

@test "commands are displayed for generations history" {
  mkdir -p "machine_a"
  mkdir -p "machine_b"

  "$FLOX_BIN" init --name "test"
  "$FLOX_BIN" push --owner owner

  # Make a few modifications
  # 1. an edit
  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1
EOF
  )"
  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f -

  # 2. an install
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.yaml" \
    "$FLOX_BIN" install hello

  # 3. switch generation, but set argv[0] to foo
  (exec -a foo "$FLOX_BIN" generations switch 1)

  run "$FLOX_BIN" generations history
  assert_line "Command:    flox push --owner owner"
  assert_line "Command:    flox edit -f -"
  assert_line "Command:    flox install hello"
  # Regardless of argv[0], we always print 'flox'
  assert_line "Command:    flox generations switch 1"
}
