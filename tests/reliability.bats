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

@test "flox should reliably use a lock in a repo without specifying a stability" {
  $FLOX_CLI flake lock --override-input flox-floxpkgs/nixpkgs/nixpkgs github:flox/nixpkgs/stable.20230603;
  before=$($FLOX_CLI eval .#hello --json )
  # simulate 30 days have passed and the lockfile updated
  $FLOX_CLI flake lock --override-input flox-floxpkgs/nixpkgs/nixpkgs github:flox/nixpkgs/stable.20230701;
  after=$($FLOX_CLI eval .#hello --json)
  echo "$before and $after should be different"
  [ "$before" != "$after" ]
}

@test "flox should use stability when specified" {
  $FLOX_CLI flake lock --override-input flox-floxpkgs/nixpkgs/nixpkgs github:flox/nixpkgs/stable.20230603;
  before=$($FLOX_CLI eval .#hello --json )
  after=$($FLOX_CLI eval .#hello --stability unstable --json)
  echo "$before and $after should be different"
  [ "$before" != "$after" ]
}

@test "flox should use only use stability when specified" {
  $FLOX_CLI flake lock --override-input flox-floxpkgs/nixpkgs/nixpkgs github:flox/nixpkgs/stable.20230603;
  before=$($FLOX_CLI eval --stability stable -v .#hello --json )
  $FLOX_CLI flake lock --override-input flox-floxpkgs/nixpkgs/nixpkgs github:flox/nixpkgs/stable.20230701;
  after=$($FLOX_CLI eval --stability stable -v .#hello --json)
  echo "$before and $after should be the same"
  [ "$before" == "$after" ]
}

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
