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

# Helpers for project based tests
# Note in this file, these aren't added to setup() and teardown()

project_setup() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/test"
  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" >/dev/null || return
  run "$FLOX_BIN" init
  assert_success
  unset output
  export MANIFEST_PATH="$PROJECT_DIR/.flox/env/manifest.toml"
}

project_teardown() {
  popd >/dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
  unset MANIFEST_PATH
}

# ---------------------------------------------------------------------------- #

setup() {
  common_test_setup
  setup_isolated_flox

  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.json"
}

teardown() {
  common_test_teardown
}

setup_file() {
  common_file_setup

  export SHOW_HINT="Use 'flox show <package>' to see available versions"
  # Separator character for ambiguous package sources
  export SEP=":"
}

# ---------------------------------------------------------------------------- #

@test "'flox search' can be called at all" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/search/hello.json"
  run "$FLOX_BIN" search hello
  assert_success
}

# ---------------------------------------------------------------------------- #

@test "'flox search' error with no search term" {
  run "$FLOX_BIN" search
  assert_failure
}

# ---------------------------------------------------------------------------- #

@test "'flox search' warns about and strips version specifiers" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/search/hello.json"
  run --separate-stderr "$FLOX_BIN" search hello@2.12.1
  assert_success
  assert_regex "$stderr" "'flox search' ignores version specifiers."
}

# ---------------------------------------------------------------------------- #

@test "'flox search' returns JSON" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/search/hello.json"
  run "$FLOX_BIN" search hello --json
  version="$(echo "$output" | jq '.[0].pname')"
  assert_equal "$version" '"hello"'
}

# ---------------------------------------------------------------------------- #

@test "'flox search' hints at 'flox show'" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/search/hello.json"
  run --separate-stderr "$FLOX_BIN" search hello
  assert_success
  assert_equal "${stderr_lines[-1]}" "$SHOW_HINT"
}

# ---------------------------------------------------------------------------- #

@test "'flox search' error message when no results" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/search/surely_doesnt_exist.json"
  run "$FLOX_BIN" search surely_doesnt_exist
  assert_equal "${#lines[@]}" 1
  assert_output --partial "No packages matched this search term: 'surely_doesnt_exist'"
}

# ---------------------------------------------------------------------------- #

@test "'flox search' accepts '--all' flag" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/search/ello_all.json"
  run "$FLOX_BIN" search --all hello
  assert_success
}

# ---------------------------------------------------------------------------- #

@test "'flox search' shows limited results when requested" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/search/python.json"
  run --separate-stderr "$FLOX_BIN" search python
  assert_success
  assert_equal "${#lines[@]}" 10 # default limit is 10 results
}

# ---------------------------------------------------------------------------- #

@test "'flox search' shows total number of results" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/search/python.json"
  run --separate-stderr "$FLOX_BIN" search python
  assert_success
  assert_regex "$stderr" '[0-9]+ of [0-9]+'
}

# ---------------------------------------------------------------------------- #

@test "'flox search' no 'X of Y' message when X=Y" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/search/exactly_ten.json"
  run --separate-stderr "$FLOX_BIN" search hello
  assert_equal "$stderr" "$SHOW_HINT"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=search:hint
@test "'flox search' includes search term in hint" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/search/python.json"
  run --separate-stderr "$FLOX_BIN" search python
  assert_regex "$stderr" "flox search python --all"
}

# bats test_tags=search:suggestions
@test "'flox search' shows suggested results" {
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/search/java_suggestions.json" \
    run "$FLOX_BIN" search java
  assert_success
  assert_output --partial "Related search results for 'jdk'"
}

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
