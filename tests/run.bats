#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test the `flox run' subcommand.
#
# ---------------------------------------------------------------------------- #

load test_support.bash;


# ---------------------------------------------------------------------------- #

@test "flox run using nixpkgs" {
  run sh -c "$FLOX_CLI run nixpkgs#cowsay -- 'Hello, world!'"
  assert_success;
  assert_output --partial - < tests/hello-cowsay.out;
}

# ---------------------------------------------------------------------------- #

@test "flox run package from project env" {
  # Note the use of --dereference to copy flake.{nix,lock} as files.
  run sh -c "tar -cf - --dereference --mode u+w -C ./tests/run ./hello | tar -C $FLOX_TEST_HOME -xf -"
  assert_success

  pushd "$FLOX_TEST_HOME/hello"
    run sh -c "$FLOX_CLI run hello"
    assert_success
    assert_output --partial "Hello"
  popd
}

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
