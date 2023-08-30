#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test flox edit
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# ---------------------------------------------------------------------------- #

# Helpers for project based tests.

project_setup() {
  # Test manifests
  export ORIGINAL_MANIFEST_CONTENTS=$(cat << EOF
{
  context,
  system,
  ...
}:
{
  packages.nixpkgs-flox.bat = { version = "0.22.1"; };
  environmentVariables.FOO = "bar";
}
EOF
  );
  export NEW_MANIFEST_CONTENTS=$(cat << EOF
{
  context,
  system,
  ...
}:
{
  packages.nixpkgs-flox.bat = { version = "0.22.1"; };
  packages.nixpkgs-flox.ripgrep = {};
  environmentVariables.FOO = "bar";
}
EOF
  );

  # These tests are not run interactively, so we should't allow the CLI to try
  # opening a text editor in the first place.
  export EDITOR=false;

  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/test";
  rm -rf "$PROJECT_DIR";
  mkdir -p "$PROJECT_DIR";
  pushd "$PROJECT_DIR" >/dev/null || return;

  "$FLOX_CLI" init
  export MANIFEST_PATH="$PROJECT_DIR/.flox/test/pkgs/default/flox.nix";
  echo "$ORIGINAL_MANIFEST_CONTENTS" > "$MANIFEST_PATH";
  export EXTERNAL_MANIFEST_PATH="$PROJECT_DIR/input.nix";
  echo "$NEW_MANIFEST_CONTENTS" > "$EXTERNAL_MANIFEST_PATH";
}

project_teardown() {
  popd >/dev/null || return;
  rm -rf "${PROJECT_DIR?}";
  unset PROJECT_DIR;
  unset ORIGINAL_MANIFEST_CONTENTS;
  unset NEW_MANIFEST_CONTENTS;
  unset MANIFEST_PATH;
}

# ---------------------------------------------------------------------------- #

setup() {
  common_test_setup;
  project_setup;
}
teardown() {
  project_teardown;
  common_test_teardown;
}

setup_file() {
  export FLOX_FEATURES_ENV=rust
}

# ---------------------------------------------------------------------------- #

check_manifest_unchanged() {
  current_contents=$(cat "$MANIFEST_PATH")
  [[ "$current_contents" = "$ORIGINAL_MANIFEST_CONTENTS" ]]
}


# ---------------------------------------------------------------------------- #

@test "'flox edit' accepts contents via filename" {
  run cat "$EXTERNAL_MANIFEST_PATH"
  run "$FLOX_CLI" edit -f "$EXTERNAL_MANIFEST_PATH";
  assert_success;
  WRITTEN=$(cat "$MANIFEST_PATH");
  assert_equal "$WRITTEN" "$NEW_MANIFEST_CONTENTS";
}


# ---------------------------------------------------------------------------- #

@test "'flox edit' accepts contents via pipe to stdin" {
  run sh -c "cat ${EXTERNAL_MANIFEST_PATH} | ${FLOX_CLI} edit -f -";
  assert_success;
  # Get the contents as they appear in the actual manifest after the operation
  WRITTEN=$(cat "$MANIFEST_PATH");
  # Assert that it's the same as the contents we supplied
  assert_equal "$WRITTEN" "$NEW_MANIFEST_CONTENTS";
}


# ---------------------------------------------------------------------------- #

@test "'flox edit' fails with invalid contents supplied via filename" {
  echo "foo = " > "$EXTERNAL_MANIFEST_PATH";
  run "$FLOX_CLI" edit -f "$EXTERNAL_MANIFEST_PATH";
  assert_failure;
  run check_manifest_unchanged;
  assert_success;
}


# ---------------------------------------------------------------------------- #

@test "'flox edit' fails with invalid contents supplied via stdin" {
  run sh -c "echo 'foo = ;' | ${FLOX_CLI} edit -f -";
  assert_failure;
  run check_manifest_unchanged;
  assert_success;
}


# ---------------------------------------------------------------------------- #

@test "'flox edit' fails when provided filename doesn't exist" {
  run "$FLOX_CLI" edit -f "does_not_exist.nix";
  assert_failure;
  run check_manifest_unchanged;
  assert_success;
}


# ---------------------------------------------------------------------------- #

@test "'flox edit' fails when EDITOR is not set" {
  run "$FLOX_CLI" edit;
  assert_failure;
  run check_manifest_unchanged;
  assert_success;
}
