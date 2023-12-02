#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# `pkgdb' basic CLI tests.
# These largely focus on things like `--help` messages, env variable handling,
# and parsers as opposed subcommand behaviors.
#
# ---------------------------------------------------------------------------- #

load setup_suite.bash;

# bats file_tags=cli


# ---------------------------------------------------------------------------- #

@test "pkgdb --help" {
  run $PKGDB --help;
  assert_success;
}


# ---------------------------------------------------------------------------- #

@test "pkgdb search --help" {
  run $PKGDB search --help;
  assert_success;
}


# ---------------------------------------------------------------------------- #

@test "pkgdb scrape --help" {
  run $PKGDB scrape --help;
  assert_success;
}


# ---------------------------------------------------------------------------- #

@test "pkgdb get --help" {
  run $PKGDB get --help;
  assert_success;
}


# ---------------------------------------------------------------------------- #

@test "pkgdb get id --help" {
  run $PKGDB get id --help;
  assert_success;
}


# ---------------------------------------------------------------------------- #

@test "pkgdb get path --help" {
  run $PKGDB get path --help;
  assert_success;
}


# ---------------------------------------------------------------------------- #

@test "pkgdb get flake --help" {
  run $PKGDB get flake --help;
  assert_success;
}


# ---------------------------------------------------------------------------- #

@test "pkgdb get db --help" {
  run $PKGDB get db --help;
  assert_success;
}


# ---------------------------------------------------------------------------- #

@test "pkgdb get done --help" {
  run $PKGDB get done --help;
  assert_success;
}


# ---------------------------------------------------------------------------- #

@test "pkgdb list --help" {
  run $PKGDB list --help;
  assert_success;
}


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
