#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test flox edit
# TODO move other edit tests from integration.bats
#
# ---------------------------------------------------------------------------- #

load test_support.bash;

# bats file_tags=edit

setup_file() {
  skip "Skipping --bash-passthru tests";
  common_file_setup;
  # If any of these tests attempt to open a text editor we want them to fail.
  # So we set the default editor to the executable `false' to ensure we
  # immediately exit with a failed status.
  # We do NOT want the test suite to hang waiting for input from `vi'!
  export EDITOR=false;
}


# Giving each test an env so that we can parallelize.
# If each test shared an env their edits may cause a race condition.
setup() {
  setup_test_envname;
  $FLOX_CLI --bash-passthru create -e "$TEST_ENVIRONMENT";
}


# ---------------------------------------------------------------------------- #

# test reading from a file
@test "'flox edit -f FILE'" {
  run $FLOX_CLI --bash-passthru edit -e "$TEST_ENVIRONMENT" -f "$TESTS_DIR/test-flox.nix";
  assert_success;
  assert_output --partial "Environment '$TEST_ENVIRONMENT' modified.";

  run sh -c "EDITOR=cat $FLOX_CLI --bash-passthru edit -e '$TEST_ENVIRONMENT';";
  assert_success;
  assert_output --partial 'environmentVariables.test = "file"';
}


# ---------------------------------------------------------------------------- #

# test reading from stdin
@test "'flox edit -f -'" {
  run sh -c "echo '{ environmentVariables.test = \"stdin\"; }' |
              $FLOX_CLI --bash-passthru edit -e '$TEST_ENVIRONMENT' --file -;";
  assert_success;
  assert_output --partial "Environment '$TEST_ENVIRONMENT' modified.";

  run sh -c "EDITOR=cat $FLOX_CLI --bash-passthru edit -e '$TEST_ENVIRONMENT';";
  assert_success;
  assert_output --partial 'environmentVariables.test = "stdin";';
}


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
