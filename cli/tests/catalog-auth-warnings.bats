#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Tests for DEV-199: per-command deprecation warnings before catalog auth
# gating is enforced.
#
# bats file_tags=catalog-auth-warnings
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# ---------------------------------------------------------------------------- #

AUTH_WARNING="This command will require authentication in an upcoming release."
LOCK_WARNING="Locking environments will require authentication in an upcoming release."

project_setup() {
  export PROJECT_NAME="test"
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/$PROJECT_NAME"
  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" > /dev/null || return
}

project_teardown() {
  popd > /dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
}

setup() {
  common_test_setup
  setup_isolated_flox
  project_setup
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.yaml"
}

teardown() {
  project_teardown
  common_test_teardown
}

# ---------------------------------------------------------------------------- #
# Unconditional warnings
# ---------------------------------------------------------------------------- #

@test "'flox init' prints auth deprecation warning" {
  run "$FLOX_BIN" init
  assert_success
  assert_output --partial "$AUTH_WARNING"
}

@test "'flox install' prints auth deprecation warning" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.yaml"
  "$FLOX_BIN" init
  run "$FLOX_BIN" install hello
  assert_success
  assert_output --partial "$AUTH_WARNING"
}

@test "'flox edit' prints auth deprecation warning" {
  "$FLOX_BIN" init
  run "$FLOX_BIN" edit -f .flox/env/manifest.toml
  assert_success
  assert_output --partial "$AUTH_WARNING"
}

@test "'flox upgrade' prints auth deprecation warning" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.yaml"
  "$FLOX_BIN" init
  "$FLOX_BIN" install hello
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.yaml"
  run "$FLOX_BIN" upgrade
  assert_success
  assert_output --partial "$AUTH_WARNING"
}

@test "'flox run' prints auth deprecation warning" {
  # The warning fires before any expensive work; use a missing-package error
  # to verify warning output without requiring a real nix store download.
  run "$FLOX_BIN" run
  # run exits with failure (no package specified), but warning is still printed
  assert_output --partial "$AUTH_WARNING"
}

@test "'flox include upgrade' prints auth deprecation warning" {
  "$FLOX_BIN" init -d included
  "$FLOX_BIN" init -d composer
  cat > composer/.flox/env/manifest.toml <<- EOF
version = 1

[include]
environments = [
  { dir = "../included" },
]
EOF
  run "$FLOX_BIN" include upgrade -d composer
  assert_success
  assert_output --partial "$AUTH_WARNING"
}

# ---------------------------------------------------------------------------- #
# Conditional warning: activate
# ---------------------------------------------------------------------------- #

@test "'flox activate' with existing lockfile does NOT print lock warning" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.yaml"
  "$FLOX_BIN" init
  "$FLOX_BIN" install hello
  # First activate creates the lockfile
  "$FLOX_BIN" activate -c true
  # Second activate: lockfile already exists — no warning expected
  run "$FLOX_BIN" activate -c true
  assert_success
  refute_output --partial "$LOCK_WARNING"
}

@test "'flox activate' without lockfile prints lock warning" {
  "$FLOX_BIN" init
  # No install or prior activate — no lockfile exists
  run "$FLOX_BIN" activate -c true
  assert_success
  assert_output --partial "$LOCK_WARNING"
}
