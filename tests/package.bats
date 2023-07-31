#!/usr/bin/env bats

load test_support.bash

setup_file() {
  common_file_setup;
  hello_pkg_setup;
  # We can't really parallelize these because we depend on past test actions.
  export BATS_NO_PARALLELIZE_WITHIN_FILE=true;
}

@test "flox install by /nix/store path" {
  run "$FLOX_CLI" install -e "$TEST_ENVIRONMENT" "$HELLO_PACKAGE"
  assert_success
  assert_output --partial "Installed '$HELLO_PACKAGE' package(s) into '$TEST_ENVIRONMENT' environment."
}

@test "flox list after installing by store path should contain package" {
  run "$FLOX_CLI" list -e "$TEST_ENVIRONMENT"
  assert_success
  assert_output --partial "Curr Gen  1"
  assert_output --partial "0  $HELLO_PACKAGE  $HELLO_PACKAGE_FIRST8"
}

@test "tear down install test state" {
  run $FLOX_CLI destroy -e "$TEST_ENVIRONMENT" --origin -f||:
  assert_output --partial "WARNING: you are about to delete the following"
  assert_output --partial "Deleted branch"
  assert_output --partial "removed"
}
