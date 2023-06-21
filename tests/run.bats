#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test the `flox run' subcommand.
#
# ---------------------------------------------------------------------------- #

load test_support.bash;


# ---------------------------------------------------------------------------- #

# We use `tar' instead of `cp' to instantiate that sandbox because Darwin
# systems are shipped with the FreeBSD implementation of system utilities -
# unlike the vastly superior GNU `coreutils' implementations, their `cp' lacks
# the ability to dereference symlinks and stuff.
setup_file() {
  common_setup;
  # Note the use of --dereference to copy flake.{nix,lock} as files.
  tar -cf - --dereference --mode u+w -C "$TESTS_DIR/run" "./hello"  \
    |tar -C "$FLOX_TEST_HOME" -xf -;
  cd "$FLOX_TEST_HOME/hello"||return;
}


# ---------------------------------------------------------------------------- #

@test "flox run using nixpkgs" {
  run $FLOX_CLI run 'nixpkgs#cowsay' -- 'Hello, world!';
  assert_success;
  assert_output --partial - < "$TESTS_DIR/hello-cowsay.out";
}


# ---------------------------------------------------------------------------- #

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
