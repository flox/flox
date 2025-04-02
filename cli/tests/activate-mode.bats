#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test activation modes.
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# bats file_tags=activate-mode

# ---------------------------------------------------------------------------- #

# Helpers for project based tests.

project_setup() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/project-${BATS_TEST_NUMBER?}"
  export PROJECT_NAME="${PROJECT_DIR##*/}"

  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" > /dev/null || return

  "$FLOX_BIN" init -d "$PROJECT_DIR"
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
}

teardown() {
  project_teardown
  common_test_teardown
}

# ---------------------------------------------------------------------------- #

@test "rejects invalid activate mode" {
  project_setup

  run "$FLOX_BIN" activate -m invalid -- true
  assert_failure
  assert_output "❌ ERROR: couldn't parse \`invalid\`: not a valid activation mode"
}

function set_manifest_mode() {
  mode="${1?}"
  tomlq --in-place -t ".options.activate.mode=\"$mode\"" .flox/env/manifest.toml
}

function assert_dev_mode() {
  assert_output --partial "${NIX_SYSTEM}.${PROJECT_NAME}.dev"
}

function assert_run_mode() {
  assert_output --partial "${NIX_SYSTEM}.${PROJECT_NAME}.run"
}

@test "activate defaults to dev mode" {
  project_setup

  run "$FLOX_BIN" activate -- printenv FLOX_ENV
  assert_success
  assert_dev_mode
}

@test "can activate in dev mode with flag" {
  project_setup

  run "$FLOX_BIN" activate -m dev -- printenv FLOX_ENV
  assert_success
  assert_dev_mode
}

@test "can activate in run mode with flag" {
  project_setup

  run "$FLOX_BIN" activate -m run -- printenv FLOX_ENV
  assert_success
  assert_run_mode
}

@test "can activate in dev mode with manifest option" {
  project_setup
  set_manifest_mode dev

  run "$FLOX_BIN" activate -- printenv FLOX_ENV
  assert_success
  assert_dev_mode
}

@test "can activate in run mode with manifest option" {
  project_setup
  set_manifest_mode run

  run "$FLOX_BIN" activate -- printenv FLOX_ENV
  assert_success
  assert_run_mode
}

@test "can activate in dev mode with flag taking precedence over manifest option" {
  project_setup
  set_manifest_mode run

  run "$FLOX_BIN" activate -m dev -- printenv FLOX_ENV
  assert_success
  assert_dev_mode
}

@test "can activate in run mode with flag taking precedence over manifest option" {
  project_setup
  set_manifest_mode dev

  run "$FLOX_BIN" activate -m run -- printenv FLOX_ENV
  assert_success
  assert_run_mode
}

@test "runtime: dev dependencies aren't added to PATH" {
  project_setup
  "$FLOX_BIN" edit -n "runtime_project" # give it a stable name
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/almonds.json" "$FLOX_BIN" install almonds
  # `almonds` brings in Python as a development dependency, and we don't want
  # that in runtime mode
  run "$FLOX_BIN" activate -m run -- bash <(cat <<'EOF'
    [ -e "$FLOX_ENV/bin/almonds" ]
    [ ! -e "$FLOX_ENV/bin/python3" ]
EOF
)
  assert_success
}

@test "runtime: packages still added to PATH" {
  project_setup
  "$FLOX_BIN" edit -n "runtime_project" # give it a stable name
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/almonds.json" "$FLOX_BIN" install almonds
  run "$FLOX_BIN" activate -m run -- which almonds
  assert_output --partial ".flox/run/$NIX_SYSTEM.runtime_project.run/bin/almonds"
}

@test "runtime: remains in runtime mode as bottom layer" {
  # Prepare two environments that we're going to layer
  export bottom_layer_dir="$BATS_TEST_TMPDIR/bottom_layer"
  mkdir "$bottom_layer_dir"
  "$FLOX_BIN" init -d "$bottom_layer_dir"
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/almonds.json" "$FLOX_BIN" install -d "$bottom_layer_dir" almonds
  export top_layer_dir="$BATS_TEST_TMPDIR/top_layer"
  mkdir "$top_layer_dir"
  "$FLOX_BIN" init -d "$top_layer_dir"
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json" "$FLOX_BIN" install -d "$top_layer_dir" hello

  run "$FLOX_BIN" activate -m run -d "$bottom_layer_dir" -- bash <(cat <<'EOF'
    set -euo pipefail

    # This is where we *would* find `python3` if it was present
    python_path_bottom="$FLOX_ENV/bin/python3"
    if [ "$(command -v python3)" = "$python_path_bottom" ]; then
      exit 1
    fi

    # Layer another environment on top
    to_eval=$("$FLOX_BIN" activate -d "$top_layer_dir")
    eval "$to_eval"

    # Ensure that we don't find Python from the bottom environment
    if [ "$(command -v python3)" = "$python_path_bottom" ]; then
      exit 1
    fi
EOF
)
  assert_success
}

@test "runtime: remains in runtime mode as top layer" {
  # Prepare two environments that we're going to layer
  export bottom_layer_dir="$BATS_TEST_TMPDIR/bottom_layer"
  mkdir "$bottom_layer_dir"
  "$FLOX_BIN" init -d "$bottom_layer_dir"
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json" "$FLOX_BIN" install -d "$bottom_layer_dir" hello
  export top_layer_dir="$BATS_TEST_TMPDIR/top_layer"
  mkdir "$top_layer_dir"
  "$FLOX_BIN" init -d "$top_layer_dir"
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/almonds.json" "$FLOX_BIN" install -d "$top_layer_dir" almonds

  run "$FLOX_BIN" activate -d "$bottom_layer_dir" -m run  -- bash <(cat <<'EOF'
    set -euo pipefail

    # Layer another environment on top
    to_eval=$("$FLOX_BIN" activate -m run -d "$top_layer_dir")
    eval "$to_eval"

    # Ensure that we don't find Python from the bottom environment
    if [ "$(command -v python3)" = "$FLOX_ENV/bin/python3" ]; then
      exit 1
    fi
EOF
)
  assert_success
}

@test "runtime: doesn't set CPATH" {
  project_setup
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json" "$FLOX_BIN" install hello
  export outer_cpath="$CPATH"
  run "$FLOX_BIN" activate -m run -- bash <(cat <<'EOF'
    [ "$CPATH" = "$outer_cpath" ]
EOF
)
  assert_success
}
