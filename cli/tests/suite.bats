#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test setup of the test suite.
#
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# bats file_tags=suite

# ---------------------------------------------------------------------------- #

@test "suite: SHELL should default to bashInteractive from Nix" {
  run printenv SHELL
  assert_success
  assert_output --regexp "^/nix/store/.*-bash-interactive-.*/bin/bash$"
}

@test "suite: FLOX_SHELL should not leak from outer user shell" {
  run printenv FLOX_SHELL
  assert_failure
}
