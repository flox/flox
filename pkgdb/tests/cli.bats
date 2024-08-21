#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# `pkgdb' basic CLI tests.
# These largely focus on things like `--help` messages, env variable handling,
# and parsers as opposed subcommand behaviors.
#
# ---------------------------------------------------------------------------- #

load setup_suite.bash

# bats file_tags=cli

# ---------------------------------------------------------------------------- #

@test "pkgdb --help" {
  run "$PKGDB_BIN" --help
  assert_success
}
