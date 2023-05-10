#!/usr/bin/env bats

bats_load_library bats-assert
bats_require_minimum_version 1.5.0

load test_support.bash

@test "flox package sanity check" {
  # directories
  [ -d $FLOX_PACKAGE/bin ]
  [ -d $FLOX_PACKAGE/libexec ]
  [ -d $FLOX_PACKAGE/libexec/flox ]
  [ -d $FLOX_PACKAGE/etc ]
  [ -d $FLOX_PACKAGE/etc/flox.zdotdir ]
  [ -d $FLOX_PACKAGE/lib ]
  [ -d $FLOX_PACKAGE/share ]
  [ -d $FLOX_PACKAGE/share/man ]
  [ -d $FLOX_PACKAGE/share/man/man1 ]
  [ -d $FLOX_PACKAGE/share/bash-completion ]
  [ -d $FLOX_PACKAGE/share/bash-completion/completions ]
  # executables
  [ -x $FLOX_CLI ]
  [ -x $FLOX_PACKAGE/libexec/flox/gh ]
  [ -x $FLOX_PACKAGE/libexec/flox/nix ]
  [ -x $FLOX_PACKAGE/libexec/flox/flox ]
  # Could go on ...
}

@test "flox --prefix" {
  run $FLOX_CLI --prefix
  assert_success
  assert_output $FLOX_PACKAGE
}

@test "flox install by /nix/store path" {
  run $FLOX_CLI install -e $TEST_ENVIRONMENT $FLOX_PACKAGE
  assert_success
  assert_output --partial "Installed '$FLOX_PACKAGE' package(s) into '$TEST_ENVIRONMENT' environment."
}

@test "flox list after installing by store path should contain package" {
  run $FLOX_CLI list -e $TEST_ENVIRONMENT
  assert_success
  assert_output --partial "Curr Gen  1"
  assert_output --partial "0  $FLOX_PACKAGE  $FLOX_PACKAGE_FIRST8"
}

@test "tear down install test state" {
  run $FLOX_CLI destroy -e $TEST_ENVIRONMENT --origin -f
  assert_output --partial "WARNING: you are about to delete the following"
  assert_output --partial "Deleted branch"
  assert_output --partial "removed"
}
