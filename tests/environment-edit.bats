#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test flox init
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# ---------------------------------------------------------------------------- #

# Helpers for project based tests.

project_setup() {
  # Manifest test inputs
  export EXISTING_MANIFEST_CONTENTS=$(cat <<EOF
{
  packages.nixpkgs-flox.bat = { version = "0.22.1"; };
  environmentVariables.FOO = "bar";
}
EOF
);
  export NEW_MANIFEST_CONTENTS=$(cat <<EOF
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
  export MANIFEST_PATH="$PROJECT_DIR/.flox/test/pkgs/default/flox.nix";
  pushd "$PROJECT_DIR" >/dev/null || return;

  "$FLOX_CLI" init
  cat "$EXISTING_MANIFEST_CONTENTS" > "$MANIFEST_PATH";
  export MANIFEST_INPUT_PATH="$PROJECT_DIR/input.nix";
  cat "$NEW_MANIFEST_CONTENTS" > "$MANIFEST_INPUT_PATH";
}

project_teardown() {
  popd >/dev/null || return;
  rm -rf "${PROJECT_DIR?}";
  unset PROJECT_DIR;
  unset EXISTING_MANIFEST_CONTENTS;
  unset UPDATED_MANIFEST_CONTENTS;
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

@test "'flox edit' accepts contents via filename" {
  skip FIXME;
}


# ---------------------------------------------------------------------------- #

@test "'flox edit' accepts contents via pipe to stdin" {
  skip FIXME;
}


# ---------------------------------------------------------------------------- #

@test "'flox edit' fails with invalid contents supplied via filename" {
  skip FIXME;
}


# ---------------------------------------------------------------------------- #

@test "'flox edit' fails when provided filename doesn't exist" {
  skip FIXME;
}


# ---------------------------------------------------------------------------- #

@test "'flox edit' fails when EDITOR is not set" {
  skip FIXME;
}


# ---------------------------------------------------------------------------- #

@test "'flox edit' updates manifest after edit; contents supplied via filename" {
  skip FIXME;
}


# ---------------------------------------------------------------------------- #

@test "'flox edit' updates manifest after edit; contents supplied via stdin" {
  skip FIXME;
}
