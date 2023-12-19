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
  run "$PKGDB_BIN" --help;
  assert_success;
}


# ---------------------------------------------------------------------------- #

@test "pkgdb search --help" {
  run "$PKGDB_BIN" search --help;
  assert_success;
}


# ---------------------------------------------------------------------------- #

@test "pkgdb scrape --help" {
  run "$PKGDB_BIN" scrape --help;
  assert_success;
}


# ---------------------------------------------------------------------------- #

@test "pkgdb get --help" {
  run "$PKGDB_BIN" get --help;
  assert_success;
}


# ---------------------------------------------------------------------------- #

@test "pkgdb get id --help" {
  run "$PKGDB_BIN" get id --help;
  assert_success;
}


# ---------------------------------------------------------------------------- #

@test "pkgdb get path --help" {
  run "$PKGDB_BIN" get path --help;
  assert_success;
}


# ---------------------------------------------------------------------------- #

@test "pkgdb get flake --help" {
  run "$PKGDB_BIN" get flake --help;
  assert_success;
}


# ---------------------------------------------------------------------------- #

@test "pkgdb get db --help" {
  run "$PKGDB_BIN" get db --help;
  assert_success;
}


# ---------------------------------------------------------------------------- #

@test "pkgdb get done --help" {
  run "$PKGDB_BIN" get done --help;
  assert_success;
}


# ---------------------------------------------------------------------------- #

@test "pkgdb list --help" {
  run "$PKGDB_BIN" list --help;
  assert_success;
}


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
