#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test the `parser-util' executable.
#
# ---------------------------------------------------------------------------- #

bats_load_library bats-support;
bats_load_library bats-assert;
bats_require_minimum_version '1.5.0';


# ---------------------------------------------------------------------------- #

# Suppress the creation of file/suite homedirs.
setup_file() { common_file_setup test; }

setup() {
  # Note the use of `-L' to copy flake.{nix,lock} as files.
  home_setup test;
  cp -LTpr -- "$TESTS_DIR/run/hello" "$FLOX_TEST_HOME/hello";
  chmod -R u+w "$FLOX_TEST_HOME/hello";
  cd "$FLOX_TEST_HOME/hello"||return;
}


# ---------------------------------------------------------------------------- #

@test "flox run using nixpkgs" {
  run $FLOX_CLI run 'nixpkgs#cowsay' -- 'Hello, world!';
  assert_success;
  assert_output --partial - < "$TESTS_DIR/hello-cowsay.out";
}


# ---------------------------------------------------------------------------- #

# XXX: If you try to run in parallel this crash failing to create `floxmeta'
@test "flox run package from project env" {
  run $FLOX_CLI run hello;
  assert_success;
  assert_output --partial "Hello";
}


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
