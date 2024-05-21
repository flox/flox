#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test flox (no subcommand) command
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# ---------------------------------------------------------------------------- #

# ---------------------------------------------------------------------------- #

setup_file() {
  common_file_setup
  export FLOX_FEATURES_USE_CATALOG=true
  export  _FLOX_USE_CATALOG_MOCK="$TESTS_DIR/catalog_responses/empty_responses.json"
}

teardown_file() {
  unset FLOX_FEATURES_USE_CATALOG
  rm "$_FLOX_USE_CATALOG_MOCK"
  unset _FLOX_USE_CATALOG_MOCK
}

setup() {
  common_test_setup
}
teardown() {
  common_test_teardown
}

@test "f1: simplify flox 'no command' info" {
  run "$FLOX_BIN"
  assert_success
  # FLOX_VERSION is set by the `flox run` command
  # and thus deviates from the expected version.
  assert_output --regexp - << EOF
flox version \d+.\d+.\d+-.+

Usage: flox OPTIONS \(init|activate|search|install|\.\.\.\) \[--help\]

Use "flox --help" for full list of commands and more information

First time\? Create an environment with "flox init"
EOF
}

@test "f?: 'flox --help' has 0 exit code" {
  run "$FLOX_BIN"
  assert_success
}

@test "f2: command grouping changes 1: 'Local Development Commands' listed in order" {
  run --separate-stderr "$FLOX_BIN" --help
  line=4
  assert_line -n "$line" --regexp '^Local Development Commands'
  line=$((line + 1))
  assert_line -n "$line" --regexp '^    init[ ]+[\w .,]+'
  line=$((line + 1))
  assert_line -n "$line" --regexp '^    activate[ ]+[\w .,]+'
  line=$((line + 1))
  assert_line -n "$line" --regexp '^    search[ ]+[\w .,]+'
  line=$((line + 1))
  assert_line -n "$line" --regexp '^    show[ ]+[\w .,]+'
  line=$((line + 1))
  assert_line -n "$line" --regexp '^    install, i[ ]+[\w .,]+'
  line=$((line + 1))
  assert_line -n "$line" --regexp '^    uninstall[ ]+[\w .,]+'
  line=$((line + 1))
  assert_line -n "$line" --regexp '^    edit[ ]+[\w .,]+'
  line=$((line + 1))
  assert_line -n "$line" --regexp '^    list[ ]+[\w .,]+'
  line=$((line + 1))
  assert_line -n "$line" --regexp '^    delete[ ]+[\w .,]+'
}

@test "f3: command grouping changes 2: 'Sharing Commands' listed in order" {
  run "$FLOX_BIN" --help
  line=14
  assert_line -n "$line" --regexp '^Sharing Commands'
  line=$((line + 1))
  assert_line -n "$line" --regexp '^    push[ ]+[\w .,]+'
  line=$((line + 1))
  assert_line -n "$line" --regexp '^    pull[ ]+[\w .,]+'
}

@test "f5: command grouping changes 3: move lesser used or not polished commands to 'Additional Commands' section with help tip." {
  run "$FLOX_BIN" --help
  assert_output --partial - << EOF
Additional Commands. Use "flox COMMAND --help" for more info
    auth, config, envs, update, upgrade
EOF
}

@test "f6: remove stability from flox --help command: Only show stability for commands that support it" {
  run "$FLOX_BIN" --help
  refute_output --partial "--stability"
}
