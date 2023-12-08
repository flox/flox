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
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/test";
  rm -rf "$PROJECT_DIR";
  mkdir -p "$PROJECT_DIR";
  pushd "$PROJECT_DIR" >/dev/null||return;
  run "$FLOX_CLI" init;
  assert_success;
  unset output;
}

project_teardown() {
  popd >/dev/null||return;
  rm -rf "${PROJECT_DIR?}";
  unset PROJECT_DIR;
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
  common_file_setup;

  export SHOW_HINT="Use \`flox show {package}\` to see available versions"
  # Separator character for ambiguous package sources
  export SEP=":";

  if [[ -z "${PKGDB_BIN}" ]]; then
    echo "You must set \$PKGDB_BIN to run these tests" >&2;
    exit 1;
  fi
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

@test "'FLOX_FEATURES_SEARCH_STRATEGY=match flox search' expected number of results: 'hello'" {
  FLOX_FEATURES_SEARCH_STRATEGY=match run --separate-stderr "$FLOX_CLI" search hello --all;
  n_lines="${#lines[@]}";
  case "$NIX_SYSTEM" in
    *-darwin)
      assert_equal "$n_lines" 11;
      assert_equal "$stderr" "$SHOW_HINT"
      ;;
    *-linux)
      assert_equal "$n_lines" 11;
      assert_equal "$stderr" "$SHOW_HINT"
      ;;
  esac
}


# ---------------------------------------------------------------------------- #

@test "'flox search' expected number of results: 'hello'" {
  run --separate-stderr "$FLOX_CLI" search hello --all;
  n_lines="${#lines[@]}";
  case "$NIX_SYSTEM" in
    *-darwin)
      assert_equal "$n_lines" 10;
      assert_equal "$stderr" "$SHOW_HINT"
      ;;
    *-linux)
      assert_equal "$n_lines" 10;
      assert_equal "$stderr" "$SHOW_HINT"
      ;;
  esac
}


# ---------------------------------------------------------------------------- #

@test "'flox search' semver search: hello@2.12.1" {
  run --separate-stderr "$FLOX_CLI" search hello@2.12.1;
  assert_equal "${#lines[@]}" 1; # 1 result
  assert_equal "${stderr_lines[0]}" "$SHOW_HINT"
}


# ---------------------------------------------------------------------------- #

@test "'flox search' returns JSON" {
  run "$FLOX_CLI" search hello --json;
  version="$(echo "$output" | jq '.[0].version')";
  assert_equal "$version" '"2.12.1"';
}


# ---------------------------------------------------------------------------- #

@test "'flox search' semver search: 'hello@>=1'" {
  run "$FLOX_CLI" search 'hello@>=1' --json;
  versions="$(echo "$output" | jq -c 'map(.version)')";
  case $THIS_SYSTEM in
    *-darwin)
      assert_equal "$versions" '["2.12.1","2.12","2.10"]';
      ;;
    *-linux)
      assert_equal "$versions" '[]';
      ;;
  esac
}


# ---------------------------------------------------------------------------- #

@test "'flox search' semver search: hello@2.x" {
  run "$FLOX_CLI" search hello@2.x --json;
  versions="$(echo "$output" | jq -c 'map(.version)')";
  assert_equal "$versions" '["2.12.1"]';
}


# ---------------------------------------------------------------------------- #

@test "'flox search' semver search: hello@=2.10" {
  run --separate-stderr "$FLOX_CLI" search hello@=2.12 --all;
  assert_equal "${#lines[@]}" 1; # 1 result
  assert_equal "${stderr_lines[0]}" "$SHOW_HINT"
}


# ---------------------------------------------------------------------------- #

@test "'flox search' semver search: hello@v2" {
  run "$FLOX_CLI" search hello@v2 --json;
  versions="$(echo "$output" | jq -c 'map(.version)')";
  assert_equal "$versions" '["2.12.1"]';
}


# ---------------------------------------------------------------------------- #

@test "'flox search' semver search: 'hello@>1 <3'" {
  run "$FLOX_CLI" search 'hello@>1 <3' --json;
  versions="$(echo "$output" | jq -c 'map(.version)')";
  assert_equal "$versions" '["2.12.1"]';
}


# ---------------------------------------------------------------------------- #

@test "'flox search' exact semver match listed first" {
  run "$FLOX_CLI" search hello@2.12.1 --json;
  first_line="$(echo "$output" | head -n 1 | grep 2.12.1)";
  assert [ -n first_line ];
}

# ---------------------------------------------------------------------------- #

@test "'flox search' hints at 'flox show'" {
  run --separate-stderr "$FLOX_CLI" search hello;
  assert_success
  assert_equal "$stderr" "$SHOW_HINT"
}


# ---------------------------------------------------------------------------- #

@test "'flox search' error message when no results" {
  run "$FLOX_CLI" search surely_doesnt_exist;
  assert_equal "${#lines[@]}" 1;
  # There's a leading `ERROR: ` that's left off when run non-interactively
  assert_output "No packages matched this search term: surely_doesnt_exist";
}

# ---------------------------------------------------------------------------- #

@test "'flox search' with 'FLOX_FEATURES_SEARCH_STRATEGY=match-name' shows fewer packages" {

  MATCH="$(FLOX_FEATURES_SEARCH_STRATEGY=match "$FLOX_CLI" search node --all | wc -l)";
  MATCH_NAME="$(FLOX_FEATURES_SEARCH_STRATEGY=match-name "$FLOX_CLI" search node --all | wc -l)";

  assert [ "$MATCH_NAME" -lt "$MATCH" ];
}

# ---------------------------------------------------------------------------- #

@test "'flox search' works in project without manifest or lockfile" {
  rm -f "$PROJECT_DIR/.flox/manifest.toml";
  run --separate-stderr "$FLOX_CLI" search hello --all;
  assert_success;
  n_lines="${#lines[@]}";
  assert_equal "$n_lines" 10; # search results from global manifest registry
}


# ---------------------------------------------------------------------------- #

@test "'flox search' works outside of projects" {
  rm -rf "$PROJECT_DIR/.flox";
  run --separate-stderr "$FLOX_CLI" search hello --all;
  assert_success;
  n_lines="${#lines[@]}";
  assert_equal "$n_lines" 10; # search results from global manifest registry
}


# ---------------------------------------------------------------------------- #

@test "'flox search' accepts '--all' flag" {
  run "$FLOX_CLI" search --all hello;
  assert_success;
}


# ---------------------------------------------------------------------------- #

@test "'flox search' shows limited results when requested" {
  # there are 700+ results for searching 'python'
  run --separate-stderr "$FLOX_CLI" search python;
  assert_success;
  assert_equal "${#lines[@]}" 10; # default limit is 10 results
}


# ---------------------------------------------------------------------------- #

@test "'flox search' shows total number of results" {
  run --separate-stderr "$FLOX_CLI" search python;
  assert_success;
  assert_regex "$stderr" '[0-9]+ of [0-9]+';
}


# ---------------------------------------------------------------------------- #

@test "'flox search' no 'X of Y' message when X=Y" {
  skip FIXME
  run --separate-stderr "$FLOX_CLI" search hello
}
# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
