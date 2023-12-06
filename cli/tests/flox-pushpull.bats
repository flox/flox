#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test `flox (push|pull)' sub-commands.
#
# ---------------------------------------------------------------------------- #

load test_support.bash;

# bats file_tags=uri, push, pull


# ---------------------------------------------------------------------------- #

setup_file() {
  skip "Skipping --bash-passthru tests";
  common_file_setup;
  "$FLOX_CLI" --bash-passthru create -e "$TEST_ENVIRONMENT";
  "$FLOX_CLI" --bash-passthru install -e "$TEST_ENVIRONMENT" hello cowsay;
  export GITHUB_TOKEN="ghx_bogus_token"
  export GH_TOKEN="$GITHUB_TOKEN"
}


# ---------------------------------------------------------------------------- #

setup()    { common_test_setup;    }
teardown() { common_test_teardown; }


# ---------------------------------------------------------------------------- #


# bats test_tags=uri:github
@test "'flox auth status'" {
  run "$FLOX_CLI" auth status;
  # N.B. the test token is a fake token for the floxtest user.
  assert_success;
  assert_output --partial "Logged in to github.com as floxtest ($FLOX_CONFIG_HOME/gh/hosts.yml)"
  assert_output --partial "Token: flox_**************"
  assert_output --partial "Token scopes: none"
}


# bats test_tags=uri:github
@test "'flox auth status --show-token'" {
  run "$FLOX_CLI" auth status --show-token;
  # N.B. the test token is a fake token for the floxtest user.
  assert_success;
  assert_output --partial "Logged in to github.com as floxtest ($FLOX_CONFIG_HOME/gh/hosts.yml)"
  assert_output --partial "Token: flox_testOAuthToken"
  assert_output --partial "Token scopes: none"
}


# bats test_tags=uri:github
@test "'flox list -e $TEST_ENVIRONMENT'" {
  # Confirm environment was created as part of setup and contains
  # the expected packages.
  run "$FLOX_CLI" --bash-passthru list -e "$TEST_ENVIRONMENT";
  assert_success;
  assert_output --partial "stable.nixpkgs-flox.cowsay";
  assert_output --partial "stable.nixpkgs-flox.hello";
}


# bats test_tags=uri:github
@test "'flox push -e $TEST_ENVIRONMENT'" {
  # Confirm we can push the environment to the gitforge.
  run "$FLOX_CLI" --bash-passthru push -e "$TEST_ENVIRONMENT";
  assert_success;
  assert_output --partial "To https://git.hub.flox.dev/floxtest/floxmeta";
  assert_output --partial "origin/$NIX_SYSTEM.$TEST_ENVIRONMENT -> $NIX_SYSTEM.$TEST_ENVIRONMENT";
}


# bats test_tags=uri:github
@test "'flox pull -e $TEST_ENVIRONMENT'" {
  # Confirm we can pull the environment from the gitforge.
  run "$FLOX_CLI" --bash-passthru pull -e "$TEST_ENVIRONMENT";
  assert_success;
  assert_output --partial "Everything up-to-date";
}


# bats test_tags=uri:github
@test "'flox destroy -e $TEST_ENVIRONMENT --origin --force'" {
  # Confirm we have privileges to destroy the environment on the origin.
  run "$FLOX_CLI" --bash-passthru destroy -e "$TEST_ENVIRONMENT" --origin --force;
  assert_success;
  assert_output --partial "Deleted branch $NIX_SYSTEM.$TEST_ENVIRONMENT";
  assert_output --partial "Deleted remote-tracking branch origin/$NIX_SYSTEM.$TEST_ENVIRONMENT";
  assert_output --partial "To https://git.hub.flox.dev/floxtest/floxmeta";
  assert_output --partial "- [deleted]";
}

# ---------------------------------------------------------------------------- #

# TODO: git+(https|ssh), tarball


# ---------------------------------------------------------------------------- #


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
