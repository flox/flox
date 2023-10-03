#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test of rust impl of `flox search`
#
# bats file_tags=search 
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
  PROJECT_REGISTRY="$BATS_TEST_DIR/search/registry.json"
  export NIX_CONFIG="flake-registry = $PWD/registry.json"
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

setup_file() {
  export FLOX_FEATURES_CHANNELS=rust;
}

# ---------------------------------------------------------------------------- #

@test "'flox search' can be called at all" {
  run "$FLOX_CLI" search hello;
  assert_success;
}


# ---------------------------------------------------------------------------- #

@test "'flox search' error with no search term" {
  run "$FLOX_CLI" search;
  assert_failure;
}


# ---------------------------------------------------------------------------- #

@test "'flox search' helpful error with unquoted redirect: hello@>1 -> hello@" {
  run "$FLOX_CLI" search hello@;
  assert_failure;
  assert_output --partial "try quoting";
}


# ---------------------------------------------------------------------------- #

@test "'flox search' expected number of results" {
  run "$FLOX_CLI" search hello;
  n_lines="${#lines[@]}";
  assert_equal "$n_lines" "4"
}


# ---------------------------------------------------------------------------- #

@test "'flox search' semver search: hello@2.10" {
  run "$FLOX_CLI" search hello@2.10;
  assert_output --partial "hello.2_10";
  n_lines="${#lines[@]}";
  assert_equal "$n_lines" "1"
}


# ---------------------------------------------------------------------------- #

@test "'flox search' semver search: 'hello@>=1'" {
  run "$FLOX_CLI" search 'hello@>=1';
  assert_output --partial "hello.latest";
  assert_output --partial "hello.2_12_1";
  assert_output --partial "hello.2_12";
  assert_output --partial "hello.2_10";
  n_lines="${#lines[@]}";
  assert_equal "$n_lines" "4"
}


# ---------------------------------------------------------------------------- #

@test "'flox search' semver search: hello@2.x" {
  run "$FLOX_CLI" search hello@2.x;
  assert_output --partial "hello.latest";
  assert_output --partial "hello.2_12_1";
  assert_output --partial "hello.2_12";
  assert_output --partial "hello.2_10";
  n_lines="${#lines[@]}";
  assert_equal "$n_lines" "4"
}


# ---------------------------------------------------------------------------- #

@test "'flox search' semver search: hello@=2.10" {
  run "$FLOX_CLI" search hello@=2.10;
  assert_output --partial "hello.2_10";
  n_lines="${#lines[@]}";
  assert_equal "$n_lines" "1"
}


# ---------------------------------------------------------------------------- #

@test "'flox search' semver search: hello@v2" {
  run "$FLOX_CLI" search hello@v2;
  assert_output --partial "hello.2_12_1";
  assert_output --partial "hello.2_12";
  assert_output --partial "hello.2_10";
  assert_output --partial "hello.latest";
  n_lines="${#lines[@]}";
  assert_equal "$n_lines" "4"
}


# ---------------------------------------------------------------------------- #

@test "'flox search' semver search: 'hello@>1 <3'" {
  run "$FLOX_CLI" search 'hello@>1 <3';
  assert_output --partial "hello.2_12_1";
  assert_output --partial "hello.2_12";
  assert_output --partial "hello.2_10";
  assert_output --partial "hello.latest";
  n_lines="${#lines[@]}";
  assert_equal "$n_lines" "4"
}


# ---------------------------------------------------------------------------- #

@test "'flox search' exact semver match listed first" {
  run "$FLOX_CLI" search hello@2.12.1;
  first_line=$(echo "$output" | head -n 1 | grep 2.12.1);
  assert [ -n first_line ];
}


# ---------------------------------------------------------------------------- #

@test "'flox search' returns JSON" {
  run "$FLOX_CLI" search hello --json;
  version=$(echo "$output" | jq '.[0].version')
  assert_equal "$version" '"2.12.1"';
}
