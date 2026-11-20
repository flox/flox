#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test of rust impl of 'flox show'
#
# bats file_tags=search,show
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# ---------------------------------------------------------------------------- #

# Helpers for project based tests.
# Note in this file, these aren't added to setup() and teardown()

project_setup() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/test"
  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" >/dev/null || return
  run "$FLOX_BIN" init
  assert_success
  unset output
  export LOCKFILE_PATH="$PROJECT_DIR/.flox/env/manifest.lock"
}

project_teardown() {
  popd >/dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
  unset LOCKFILE_PATH
}

# ---------------------------------------------------------------------------- #

setup() {
  common_test_setup
  setup_isolated_flox
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.yaml"
}

teardown() {
  common_test_teardown
}

setup_file() {
  :
}

# ---------------------------------------------------------------------------- #

@test "'flox show' can be called at all" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/show/hello.yaml"
  run "$FLOX_BIN" show hello
  assert_success
}

# ---------------------------------------------------------------------------- #

@test "'flox show' accepts search output without separator" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA//search/hello.yaml"
  run "$FLOX_BIN" search hello
  assert_success
  first_result="${lines[0]%% *}"
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/show/hello.yaml"
  run "$FLOX_BIN" show "$first_result"
  assert_success
}

# ---------------------------------------------------------------------------- #

@test "'flox show' - hello" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/show/hello.yaml"
  run "$FLOX_BIN" show hello
  assert_success
  assert_equal "${lines[0]}" "hello - Program that produces a familiar, friendly greeting"
  assert_equal "${lines[1]}" "Catalog: nixpkgs"
  assert_equal "${lines[2]}" "Latest:  hello@2.12.3"
  assert_equal "${lines[3]}" "License: GPL-3.0-or-later"
  assert_equal "${lines[4]}" "Outputs: out* (* installed by default)"
  assert_equal "${lines[5]}" "Systems: aarch64-darwin, aarch64-linux, x86_64-darwin, x86_64-linux"
  assert_equal "${lines[6]}" "Other versions:"
  assert_equal "${lines[7]}" "    hello@2.12.3"
  assert_equal "${lines[8]}" "    hello@2.12.2"
  assert_equal "${lines[9]}" "    hello@2.12.1"
  assert_equal "${lines[10]}" "    hello@2.12"
  assert_equal "${lines[11]}" "    hello@2.10"
}

@test "'flox show' - no deprecation warning for a package that isn't deprecated" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/show/hello.yaml"
  run --separate-stderr "$FLOX_BIN" show hello
  assert_success
  assert_equal "$stderr" ""
}

# ---------------------------------------------------------------------------- #

# The catalog reports `php81` as deprecated with a message and no replacement.
@test "'flox show' - deprecated package without a replacement" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/show/php81.yaml"
  run --separate-stderr "$FLOX_BIN" show php81
  assert_success
  assert_equal "$stderr" "! php81 is EOL"
  assert_equal "${lines[0]}" "php81 - HTML-embedded scripting language"
  assert_equal "${lines[1]}" "Catalog: nixpkgs"
  assert_equal "${lines[2]}" "Latest:  php81@8.1.33"

  # The warning is after the version list, which can run to many screens, so
  # a user at the end of the output sees it. This is the output as a terminal
  # shows it, with stdout and stderr merged.
  run "$FLOX_BIN" show php81
  assert_success
  assert_equal "${lines[-2]}" "    php81@8.1.1"
  assert_equal "${lines[-1]}" "! php81 is EOL"
}

# The catalog reports `aria` as renamed to `aria2`.
@test "'flox show' - deprecated package with a replacement" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/show/aria.yaml"
  run --separate-stderr "$FLOX_BIN" show aria
  assert_success
  assert_equal "$stderr" "! 'aria' has been renamed to/replaced by 'aria2'"
  assert_equal "${lines[0]}" "aria - Lightweight, multi-protocol, multi-source, command-line download utility"
  assert_equal "${lines[2]}" "Latest:  aria@aria2-1.37.0"

  # The warning is the last line of the output as a terminal shows it, with
  # stdout and stderr merged.
  run "$FLOX_BIN" show aria
  assert_success
  assert_equal "${lines[-2]}" "    aria@aria2-1.35.0"
  assert_equal "${lines[-1]}" "! 'aria' has been renamed to/replaced by 'aria2'"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=python

# Check pkg-path is handled correctly
@test "'flox show' - python310Packages.flask" {
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/show/flask.yaml" \
    run "$FLOX_BIN" show python310Packages.flask
  assert_success
  # Ensure that the package and part of the description show up
  assert_output --partial 'python310Packages.flask - The'
}

# ---------------------------------------------------------------------------- #
