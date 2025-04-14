
#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test environment composition
#
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# bats file_tags=compose

# ---------------------------------------------------------------------------- #

setup_file() {
  common_file_setup
}

setup() {
  common_test_setup
  home_setup test # Isolate $HOME for each test.
  setup_isolated_flox
  project_setup

  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.json"
}

teardown() {
  # fifo is in PROJECT_DIR and keeps watchdog running,
  # so cat_teardown_fifo must be run before wait_for_watchdogs and
  # project_teardown
  cat_teardown_fifo
  # Cleaning up the `BATS_TEST_TMPDIR` occasionally fails,
  # because of an 'env-registry.json' that gets concurrently written
  # by the watchdog as the activation terminates.
  if [ -n "${PROJECT_DIR:-}" ]; then
    # Not all tests call project_setup
    wait_for_watchdogs "$PROJECT_DIR" || return 1
    project_teardown
  fi
  common_test_teardown
}

# ---------------------------------------------------------------------------- #

# Helpers for project based tests.

project_setup() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/project-${BATS_TEST_NUMBER?}"
  export PROJECT_NAME="${PROJECT_DIR##*/}"

  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" >/dev/null || return

}

project_teardown() {
  popd >/dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
  unset PROJECT_NAME
}

# ---------------------------------------------------------------------------- #
# Tests that share some helpers for setting up a composer and included
# environments
# ---------------------------------------------------------------------------- #

setup_composer_and_two_includes() {
  # Setup included1 environment
  "$FLOX_BIN" init -d included1
  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1

    [vars]
    included1 = "v1"
EOF
  )"
  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f - -d included1

  # Setup included2 environment
  "$FLOX_BIN" init -d included2
  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1

    [vars]
    included2 = "v1"
EOF
  )"
  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f - -d included2

  # Setup composer
  "$FLOX_BIN" init -d composer
  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1

    [include]
    environments = [
      { dir = "../included1" },
      { dir = "../included2" },
    ]
EOF
  )"
  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f - -d composer
}

# Modify vars.included1 in environment included1
edit_included1() {
  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1

    [vars]
    included1 = "v2"
EOF
  )"
  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f - -d included1
}

edit_both_included_environments() {
  edit_included1

  # Edit included2
  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1

    [vars]
    included2 = "v2"
EOF
  )"
  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f - -d included2

}

@test "include upgrade reports no changes" {
  setup_composer_and_two_includes
  run "$FLOX_BIN" include upgrade -d composer
  assert_success
  assert_output "ℹ️  No included environments have changes."
}

@test "include upgrade reports no changes when non-upgraded environment changes" {
  setup_composer_and_two_includes
  edit_included1
  run "$FLOX_BIN" include upgrade -d composer included2
  assert_success
  assert_output "ℹ️  Included environment 'included2' has no changes."
}

@test "include upgrade defaults to upgrading all" {
  setup_composer_and_two_includes
  edit_both_included_environments

  run "$FLOX_BIN" include upgrade -d composer
  assert_success
  assert_output - <<EOF
✅ Upgraded 'composer' with latest changes to:
- 'included1'
- 'included2'
EOF

  run "$FLOX_BIN" list -c -d composer
  assert_success
  assert_output --partial 'included1 = "v2"'
  assert_output --partial 'included2 = "v2"'
}

@test "include upgrade can get latest changes for a single included environment" {
  setup_composer_and_two_includes
  edit_both_included_environments

  run "$FLOX_BIN" include upgrade -d composer included1
  assert_success
  assert_output - <<EOF
✅ Upgraded 'composer' with latest changes to:
- 'included1'
EOF

  run "$FLOX_BIN" list -c -d composer
  assert_success
  assert_output --partial 'included1 = "v2"'
  assert_output --partial 'included2 = "v1"'
}

@test "include upgrade reports which included environments have changes" {
  setup_composer_and_two_includes
  edit_included1

  run "$FLOX_BIN" include upgrade -d composer included1 included2
  assert_success
  assert_output - <<EOF
✅ Upgraded 'composer' with latest changes to:
- 'included1'
ℹ️  Included environment 'included2' has no changes.
EOF

  run "$FLOX_BIN" list -c -d composer
  assert_success
  assert_output --partial 'included1 = "v2"'
  assert_output --partial 'included2 = "v1"'
}

# ---------------------------------------------------------------------------- #

function setup_composer_with_remote_include() {
  floxhub_setup owner

  # Setup owner/remote environment
  "$FLOX_BIN" init -d remote
  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1

    [vars]
    remote = "v1"
EOF
  )"
  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f - -d remote
  "$FLOX_BIN" push -d remote --owner "$OWNER"
  rm -rf remote

  # Setup composer
  "$FLOX_BIN" init -d composer
  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1

    [include]
    environments = [
      { remote = "owner/remote" },
    ]
EOF
  )"
  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f - -d composer
}

function edit_remote() {
  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1

    [vars]
    remote = "v2"
EOF
  )"
  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f - -r owner/remote
}

@test "include upgrade reports no changes for remote environments" {
  setup_composer_with_remote_include
  run "$FLOX_BIN" include upgrade -d composer
  assert_success
  assert_output - <<EOF
⚠️  Using file://${FLOX_FLOXHUB_PATH} as FloxHub host
'\$_FLOX_FLOXHUB_GIT_URL' is used for testing purposes only,
alternative FloxHub hosts are not yet supported!

ℹ️  No included environments have changes.
EOF
}

@test "include upgrade reports which remote environments have changes" {
  setup_composer_with_remote_include
  edit_remote
  run "$FLOX_BIN" include upgrade -d composer
  assert_success
  assert_output - <<EOF
⚠️  Using file://${FLOX_FLOXHUB_PATH} as FloxHub host
'\$_FLOX_FLOXHUB_GIT_URL' is used for testing purposes only,
alternative FloxHub hosts are not yet supported!

✅ Upgraded 'composer' with latest changes to:
- 'remote'
EOF
}

# ---------------------------------------------------------------------------- #
