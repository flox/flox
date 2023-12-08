#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test the `flox run' subcommand.
#
# ---------------------------------------------------------------------------- #

load test_support.bash;

# bats file_tags=run


# ---------------------------------------------------------------------------- #

# Suppress the creation of file/suite homedirs.
setup_file() {
  skip "list deprecated";
  common_file_setup test;
}

setup() {
  # Note the use of `-L' to copy flake.{nix,lock} as files.
  home_setup test;
  cp -LTpr -- "$TESTS_DIR/run/hello" "$FLOX_TEST_HOME/hello";
  chmod -R u+w "$FLOX_TEST_HOME/hello";
  cd "$FLOX_TEST_HOME/hello"||return;
}


# ---------------------------------------------------------------------------- #

@test "flox run using nixpkgs" {
  run "$FLOX_BIN" run 'nixpkgs#cowsay' -- 'Hello, world!';
  assert_success;
  assert_output --partial - < "$TESTS_DIR/hello-cowsay.out";
}


# ---------------------------------------------------------------------------- #

# XXX: If you try to run in parallel this crash failing to create `floxmeta'
@test "flox run package from project env" {
  run "$FLOX_BIN" run hello;
  assert_success;
  assert_output --partial "Hello";
}


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
