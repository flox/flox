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

project_setup() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/test"
  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" >/dev/null || return
  run "$FLOX_BIN" init;
  assert_success;
  unset output;
}

project_teardown() {
  popd >/dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
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

# ---------------------------------------------------------------------------- #

@test "'flox show' can be called at all" {
  run "$FLOX_BIN" show hello;
  assert_success;
}


# ---------------------------------------------------------------------------- #

@test "'flox show' accepts specific input" {
  skip DEPRECATED;
  run "$FLOX_BIN" show nixpkgs-flox:hello;
  assert_success;
  # TODO: better testing once the formatting is implemented
}

# ---------------------------------------------------------------------------- #

@test "'flox show' accepts search output without separator" {
  run "$FLOX_BIN" search hello;
  assert_success;
  first_result="${lines[0]%% *}";
  run "$FLOX_BIN" show "$first_result";
  assert_success;
}


# ---------------------------------------------------------------------------- #

@test "'flox show' accepts search output with separator" {
  skip DEPRECATED;
  run "$FLOX_BIN" search nixpkgs-flox:hello;
  assert_success;
  first_result="${lines[0]%% *}";
  run "$FLOX_BIN" show "$first_result";
  assert_success;
}


# ---------------------------------------------------------------------------- #

@test "'flox show' - hello" {
  run "$FLOX_BIN" show hello;
  assert_success;
  assert_equal "${lines[0]}" "hello - A program that produces a familiar, friendly greeting";
  assert_equal "${lines[1]}" "    hello - hello@2.12.1";
}


# ---------------------------------------------------------------------------- #

@test "'flox show' - hello --all" {
  run "$FLOX_BIN" show hello --all;
  assert_success;
  assert_equal "${lines[0]}" "hello - A program that produces a familiar, friendly greeting";
  assert_equal "${lines[1]}" "    hello - hello@2.12.1";
}


# ---------------------------------------------------------------------------- #

@test "'flox show' - python27Full" {
  run "$FLOX_BIN" show python27Full;
  assert_success;
  assert_equal "${lines[0]}" "python27Full - A high-level dynamically-typed programming language";
  assert_equal "${lines[1]}" "    python27Full - python27Full@2.7.18.6";
}


# ---------------------------------------------------------------------------- #

@test "'flox show' - python27Full --all" {
  run "$FLOX_BIN" show python27Full --all;
  assert_success;
  assert_equal "${lines[0]}" "python27Full - A high-level dynamically-typed programming language";
  assert_equal "${lines[1]}" "    python27Full - python27Full@2.7.18.6";
}


# ---------------------------------------------------------------------------- #

@test "'flox show' works in project without manifest or lockfile" {
  rm -f "$PROJECT_DIR/.flox/manifest.toml";
  run --separate-stderr "$FLOX_BIN" show hello;
  assert_success;
}


# ---------------------------------------------------------------------------- #

@test "'flox show' works outside of projects" {
  rm -rf "$PROJECT_DIR/.flox";
  run --separate-stderr "$FLOX_BIN" show hello;
  assert_success;
}
