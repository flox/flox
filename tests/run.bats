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

# We use `tar' instead of `cp' to instantiate that sandbox because Darwin
# systems are shipped with the FreeBSD implementation of system utilities -
# unlike the vastly superior GNU `coreutils' implementations, their `cp' lacks
# the ability to dereference symlinks and stuff.
setup_file() {
  common_setup;
  # Note the use of --dereference to copy flake.{nix,lock} as files.
  tar -cf - --dereference --mode u+w -C "$TESTS_DIR/run" "./hello"  \
    |tar -C "$FLOX_TEST_HOME" -xf -;
  # We can't really parallelize these because we depend on past test actions.
  export BATS_NO_PARALLELIZE_WITHIN_FILE=true;
}

setup() {
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
