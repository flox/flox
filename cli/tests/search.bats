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
  pushd "$PROJECT_DIR" > /dev/null || return
  run "$FLOX_BIN" init
  assert_success
  unset output
  export MANIFEST_PATH="$PROJECT_DIR/.flox/env/manifest.toml"
}

project_teardown() {
  popd > /dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
  unset MANIFEST_PATH
}

# ---------------------------------------------------------------------------- #

setup() {
  common_test_setup
  setup_isolated_flox

  _PKGDB_GA_REGISTRY_REF_OR_REV="$PKGDB_NIXPKGS_REV_OLD" \
    "$FLOX_BIN" update --global

  export FLOX_FEATURES_USE_CATALOG=true
  export  _FLOX_USE_CATALOG_MOCK="$TEST_DATA_DIR/empty.json"
}

teardown() {
  common_test_teardown
}

setup_file() {
  common_file_setup

  export SHOW_HINT="Use 'flox show <package>' to see available versions"
  # Separator character for ambiguous package sources
  export SEP=":"

  if [[ -z "${PKGDB_BIN}" ]]; then
    echo "You must set \$PKGDB_BIN to run these tests" >&2
    exit 1
  fi
}

# ---------------------------------------------------------------------------- #

@test "'flox search' can be called at all" {
  unset FLOX_FEATURES_USE_CATALOG
  run "$FLOX_BIN" search hello
  assert_success
}

# ---------------------------------------------------------------------------- #

@test "catalog: 'flox search' can be called at all" {
  export  _FLOX_USE_CATALOG_MOCK="$TEST_DATA_DIR/search/hello.json"
  run "$FLOX_BIN" search hello
  assert_success
}

# ---------------------------------------------------------------------------- #

@test "'flox search' error with no search term" {
  run "$FLOX_BIN" search
  assert_failure
}

# ---------------------------------------------------------------------------- #

@test "'flox search' helpful error with unquoted redirect: hello@>1 -> hello@" {
  unset FLOX_FEATURES_USE_CATALOG
  run "$FLOX_BIN" search hello@
  assert_failure
  assert_output --partial "try quoting"
}

# ---------------------------------------------------------------------------- #

@test "catalog: 'flox search' helpful error with unquoted redirect: hello@>1 -> hello@" {
  skip "semver search not yet supported by catalog"
  run "$FLOX_BIN" search hello@
  assert_failure
  assert_output --partial "try quoting"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=search:match-stategy
@test "'FLOX_FEATURES_SEARCH_STRATEGY=match flox search' expected number of results: 'hello'" {
  unset FLOX_FEATURES_USE_CATALOG
  FLOX_FEATURES_SEARCH_STRATEGY=match run --separate-stderr "$FLOX_BIN" search hello --all
  assert_equal "${#lines[@]}" 11
  assert_equal "$stderr" "$SHOW_HINT"
}

# ---------------------------------------------------------------------------- #

@test "'flox search' expected number of results: 'hello'" {
  unset FLOX_FEATURES_USE_CATALOG
  run --separate-stderr "$FLOX_BIN" search hello --all
  assert_equal "${#lines[@]}" 10
  assert_equal "$stderr" "$SHOW_HINT"
}

# ---------------------------------------------------------------------------- #

@test "'flox search' semver search: hello@2.12.1" {
  unset FLOX_FEATURES_USE_CATALOG
  run --separate-stderr "$FLOX_BIN" search hello@2.12.1
  assert_equal "${#lines[@]}" 1 # 1 result
  assert_equal "${stderr_lines[0]}" "$SHOW_HINT"
}

# ---------------------------------------------------------------------------- #

@test "catalog: 'flox search' semver search: hello@2.12.1" {
  skip "semver search not yet supported by catalog"
  run --separate-stderr "$FLOX_BIN" search hello@2.12.1
  assert_equal "${#lines[@]}" 1 # 1 result
  assert_equal "${stderr_lines[0]}" "$SHOW_HINT"
}

# ---------------------------------------------------------------------------- #

@test "'flox search' returns JSON" {
  unset FLOX_FEATURES_USE_CATALOG
  run "$FLOX_BIN" search hello --json
  version="$(echo "$output" | jq '.[0].version')"
  assert_equal "$version" '"2.12.1"'
}

# ---------------------------------------------------------------------------- #

@test "catalog: 'flox search' returns JSON" {
  export  _FLOX_USE_CATALOG_MOCK="$TEST_DATA_DIR/search/hello.json"
  run "$FLOX_BIN" search hello --json
  version="$(echo "$output" | jq '.[0].version')"
  assert_equal "$version" '"2.12.1"'
}

# ---------------------------------------------------------------------------- #

@test "'flox search' semver search: 'hello@>=1'" {
  unset FLOX_FEATURES_USE_CATALOG
  run "$FLOX_BIN" search 'hello@>=1' --json
  versions="$(echo "$output" | jq -c 'map(.version)')"
  case "$THIS_SYSTEM" in
    *-darwin)
      assert_equal "$versions" '["2.12.1","2.12","2.10"]'
      ;;
    *-linux)
      assert_equal "$versions" '[]'
      ;;
  esac
}

# ---------------------------------------------------------------------------- #

@test "catalog: 'flox search' semver search: 'hello@>=1'" {
  skip "semver search not yet supported by catalog"
  run "$FLOX_BIN" search 'hello@>=1' --json
  versions="$(echo "$output" | jq -c 'map(.version)')"
  case "$THIS_SYSTEM" in
    *-darwin)
      assert_equal "$versions" '["2.12.1","2.12","2.10"]'
      ;;
    *-linux)
      assert_equal "$versions" '[]'
      ;;
  esac
}

# ---------------------------------------------------------------------------- #

@test "'flox search' semver search: hello@2.x" {
  unset FLOX_FEATURES_USE_CATALOG
  run "$FLOX_BIN" search hello@2.x --json
  versions="$(echo "$output" | jq -c 'map(.version)')"
  assert_equal "$versions" '["2.12.1"]'
}

# ---------------------------------------------------------------------------- #

@test "catalog: 'flox search' semver search: hello@2.x" {
  skip "semver search not yet supported by catalog"
  run "$FLOX_BIN" search hello@2.x --json
  versions="$(echo "$output" | jq -c 'map(.version)')"
  assert_equal "$versions" '["2.12.1"]'
}

# ---------------------------------------------------------------------------- #

@test "'flox search' semver search: hello@=2.10" {
  unset FLOX_FEATURES_USE_CATALOG
  run --separate-stderr "$FLOX_BIN" search hello@=2.12 --all
  assert_equal "${#lines[@]}" 1 # 1 result
  assert_equal "${stderr_lines[0]}" "$SHOW_HINT"
}

# ---------------------------------------------------------------------------- #

@test "catalog: 'flox search' semver search: hello@=2.10" {
  skip "semver search not yet supported by catalog"
  run --separate-stderr "$FLOX_BIN" search hello@=2.12 --all
  assert_equal "${#lines[@]}" 1 # 1 result
  assert_equal "${stderr_lines[0]}" "$SHOW_HINT"
}

# ---------------------------------------------------------------------------- #

@test "'flox search' semver search: hello@v2" {
  unset FLOX_FEATURES_USE_CATALOG
  run "$FLOX_BIN" search hello@v2 --json
  versions="$(echo "$output" | jq -c 'map(.version)')"
  assert_equal "$versions" '["2.12.1"]'
}

# ---------------------------------------------------------------------------- #

@test "catalog: 'flox search' semver search: hello@v2" {
  skip "semver search not yet supported by catalog"
  run "$FLOX_BIN" search hello@v2 --json
  versions="$(echo "$output" | jq -c 'map(.version)')"
  assert_equal "$versions" '["2.12.1"]'
}

# ---------------------------------------------------------------------------- #

@test "'flox search' semver search: 'hello@>1 <3'" {
  unset FLOX_FEATURES_USE_CATALOG
  run "$FLOX_BIN" search 'hello@>1 <3' --json
  versions="$(echo "$output" | jq -c 'map(.version)')"
  assert_equal "$versions" '["2.12.1"]'
}

# ---------------------------------------------------------------------------- #

@test "catalog: 'flox search' semver search: 'hello@>1 <3'" {
  skip "semver search not yet supported by catalog"
  run "$FLOX_BIN" search 'hello@>1 <3' --json
  versions="$(echo "$output" | jq -c 'map(.version)')"
  assert_equal "$versions" '["2.12.1"]'
}

# ---------------------------------------------------------------------------- #

@test "'flox search' exact semver match listed first" {
  unset FLOX_FEATURES_USE_CATALOG
  run "$FLOX_BIN" search hello@2.12.1 --json
  first_line="$(echo "$output" | head -n 1 | grep 2.12.1)"
  assert [ -n first_line ]
}

# ---------------------------------------------------------------------------- #

@test "catalog: 'flox search' exact semver match listed first" {
  skip "semver search not yet supported by catalog"
  run "$FLOX_BIN" search hello@2.12.1 --json
  first_line="$(echo "$output" | head -n 1 | grep 2.12.1)"
  assert [ -n first_line ]
}

# ---------------------------------------------------------------------------- #

@test "'flox search' hints at 'flox show'" {
  unset FLOX_FEATURES_USE_CATALOG
  run --separate-stderr "$FLOX_BIN" search hello
  assert_success
  assert_equal "$stderr" "$SHOW_HINT"
}

# ---------------------------------------------------------------------------- #

@test "catalog: 'flox search' hints at 'flox show'" {
  export  _FLOX_USE_CATALOG_MOCK="$TEST_DATA_DIR/search/hello.json"
  run --separate-stderr "$FLOX_BIN" search hello
  assert_success
  assert_equal "$stderr" "$SHOW_HINT"
}

# ---------------------------------------------------------------------------- #

@test "'flox search' error message when no results" {
  unset FLOX_FEATURES_USE_CATALOG
  run "$FLOX_BIN" search surely_doesnt_exist
  assert_equal "${#lines[@]}" 1
  assert_output --partial "No packages matched this search term: 'surely_doesnt_exist'"
}

# ---------------------------------------------------------------------------- #

@test "catalog: 'flox search' error message when no results" {
  export  _FLOX_USE_CATALOG_MOCK="$TEST_DATA_DIR/search/surely_doesnt_exist.json"
  run "$FLOX_BIN" search surely_doesnt_exist
  assert_equal "${#lines[@]}" 1
  assert_output --partial "No packages matched this search term: 'surely_doesnt_exist'"
}

# ---------------------------------------------------------------------------- #

@test "'flox search' with 'FLOX_FEATURES_SEARCH_STRATEGY=match-name' shows fewer packages" {
  unset FLOX_FEATURES_USE_CATALOG

  MATCH="$(FLOX_FEATURES_SEARCH_STRATEGY=match "$FLOX_BIN" search node --all | wc -l)"
  MATCH_NAME="$(FLOX_FEATURES_SEARCH_STRATEGY=match-name "$FLOX_BIN" search node --all | wc -l)"

  assert [ "$MATCH_NAME" -lt "$MATCH" ]
}

# ---------------------------------------------------------------------------- #

@test "'flox search' works in project without manifest or lockfile" {
  unset FLOX_FEATURES_USE_CATALOG
  project_setup

  rm -f "$PROJECT_DIR/.flox/manifest.toml"
  run --separate-stderr "$FLOX_BIN" search hello --all
  assert_success
  n_lines="${#lines[@]}"
  assert_equal "$n_lines" 10 # search results from global manifest registry

  project_teardown
}

# ---------------------------------------------------------------------------- #

@test "'flox search' accepts '--all' flag" {
  unset FLOX_FEATURES_USE_CATALOG
  run "$FLOX_BIN" search --all hello
  assert_success
}

# ---------------------------------------------------------------------------- #

@test "catalog: 'flox search' accepts '--all' flag" {
  export  _FLOX_USE_CATALOG_MOCK="$TEST_DATA_DIR/search/ello_all.json"
  run "$FLOX_BIN" search --all hello
  assert_success
}

# ---------------------------------------------------------------------------- #

@test "'flox search' shows limited results when requested" {
  unset FLOX_FEATURES_USE_CATALOG
  # there are 700+ results for searching 'python'
  run --separate-stderr "$FLOX_BIN" search python
  assert_success
  assert_equal "${#lines[@]}" 10 # default limit is 10 results
}

# ---------------------------------------------------------------------------- #

@test "catalog: 'flox search' shows limited results when requested" {
  export  _FLOX_USE_CATALOG_MOCK="$TEST_DATA_DIR/search/python.json"
  run --separate-stderr "$FLOX_BIN" search python
  assert_success
  assert_equal "${#lines[@]}" 10 # default limit is 10 results
}

# ---------------------------------------------------------------------------- #

@test "'flox search' shows total number of results" {
  unset FLOX_FEATURES_USE_CATALOG
  run --separate-stderr "$FLOX_BIN" search python
  assert_success
  assert_regex "$stderr" '[0-9]+ of [0-9]+'
}

# ---------------------------------------------------------------------------- #

@test "catalog: 'flox search' shows total number of results" {
  export  _FLOX_USE_CATALOG_MOCK="$TEST_DATA_DIR/search/python.json"
  run --separate-stderr "$FLOX_BIN" search python
  assert_success
  assert_regex "$stderr" '[0-9]+ of [0-9]+'
}

# ---------------------------------------------------------------------------- #

@test "'flox search' no 'X of Y' message when X=Y" {
  unset FLOX_FEATURES_USE_CATALOG
  # There are exactly 10 results for 'hello' on our current nixpkgs rev
  # when search with `match-name`
  run --separate-stderr "$FLOX_BIN" search hello
  assert_equal "$stderr" "$SHOW_HINT"
}

# ---------------------------------------------------------------------------- #

@test "catalog: 'flox search' no 'X of Y' message when X=Y" {
  export  _FLOX_USE_CATALOG_MOCK="$TEST_DATA_DIR/search/exactly_ten.json"
  run --separate-stderr "$FLOX_BIN" search hello
  assert_equal "$stderr" "$SHOW_HINT"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=search:hint
@test "'flox search' includes search term in hint" {
  unset FLOX_FEATURES_USE_CATALOG
  run --separate-stderr "$FLOX_BIN" search python
  assert_regex "$stderr" "flox search python --all"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=search:hint
@test "catalog: 'flox search' includes search term in hint" {
  export  _FLOX_USE_CATALOG_MOCK="$TEST_DATA_DIR/search/python.json"
  run --separate-stderr "$FLOX_BIN" search python
  assert_regex "$stderr" "flox search python --all"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=python

@test "'flox search' - python310Packages.flask" {
  unset FLOX_FEATURES_USE_CATALOG
  run "$FLOX_BIN" search python310Packages.flask
  assert_success
  # Ensure that the package and part of the description show up
  assert_output --partial 'python310Packages.flask'
}

# ---------------------------------------------------------------------------- #

# bats test_tags=ruby

@test "'flox search' - rubyPackages.rails" {
  unset FLOX_FEATURES_USE_CATALOG
  run "$FLOX_BIN" search rubyPackages.rails
  assert_success
  assert_output --partial 'rubyPackages.rails'
}

# ---------------------------------------------------------------------------- #

# bats test_tags=python

@test "'flox search' - python310Packages" {
  unset FLOX_FEATURES_USE_CATALOG
  run "$FLOX_BIN" search python310Packages
  assert_success
  assert_output --partial 'Showing 10 of'
}

# ---------------------------------------------------------------------------- #

# bats test_tags=python

@test "'flox search' - Packages.req" {
  unset FLOX_FEATURES_USE_CATALOG
  run "$FLOX_BIN" search Packages.req
  assert_success
  assert_output --partial 'Showing 10 of'
}

# ---------------------------------------------------------------------------- #

@test "'flox search' - prints pkg-path" {
  export  _FLOX_USE_CATALOG_MOCK="$TEST_DATA_DIR/search/hello.json"
  run --separate-stderr "$FLOX_BIN" search hello
  assert_success
  assert_output --partial 'texlivePackages.othello'
}

# ---------------------------------------------------------------------------- #

@test "'flox search' - same number of results for single and multi-system environments" {
  unset FLOX_FEATURES_USE_CATALOG
  project_setup

  local extra_system
  run --separate-stderr "$FLOX_BIN" search neovim
  assert_success

  num_lines="${#lines[@]}"

  # extract total from '* of XX results*'
  total="${stderr#* of }"
  total="${total% results*}"

  # add a second system to search in
  tomlq -i -t ".options.systems += [ \"$(get_system_other_than_current)\" ]" "$MANIFEST_PATH"
  run --separate-stderr "$FLOX_BIN" search neovim
  assert_success

  multi_system_num_lines="${#lines[@]}"

  # extract showing from '*Showing XX of*
  multi_system_showing="${stderr#*Showing }"
  multi_system_showing="${multi_system_showing% of*}"

  # extract total from '* of XX results*'
  multi_system_total="${stderr#* of }"
  multi_system_total="${multi_system_total% results*}"

  # We should be displaying the number of lines we say we are.
  assert_equal "$multi_system_num_lines" "$multi_system_showing"
  # We should be displaying the default limit of 10 lines.
  assert_equal "$multi_system_num_lines" 10
  # We should have the same numbers before and after adding the second system.
  assert_equal "$num_lines" "$multi_system_num_lines"
  assert_equal "$total" "$multi_system_total"

  project_teardown
}

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
