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
setup_file() { common_file_setup test; }

setup() {
  # Note the use of `-L' to copy flake.{nix,lock} as files.
  home_setup test;
  cp -LTpr -- "$TESTS_DIR/run/hello" "$FLOX_TEST_HOME/hello";
  chmod -R u+w "$FLOX_TEST_HOME/hello";
  cd "$FLOX_TEST_HOME/hello"||return;
}


# ---------------------------------------------------------------------------- #

@test "flox should reliably use a lock in a repo" {
  $FLOX_CLI flake lock --override-input flox-floxpkgs/nixpkgs/nixpkgs github:flox/nixpkgs/stable.20230603;
  before=$($FLOX_CLI eval .#hello --json )
  # simulate 30 days have passed
  $FLOX_CLI flake lock --override-input flox-floxpkgs/nixpkgs/nixpkgs github:flox/nixpkgs/stable.20230701;
  after=$($FLOX_CLI eval .#hello --json)
  echo "$before and $after should be different"
  [ "$before" != "$after" ]
}

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
