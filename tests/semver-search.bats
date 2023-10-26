#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test the `flox search' command, specifically for use with semver ranges.
# This test includes a few regular search tests as a baseline to ensure the
# semver parser isn't wrongly firing.
#
# XXX:
# This test depends on the repo `github:flox-examples/nixpkgs-netlify' and
# expects certain versions of `nodejs' to be available.
# If that repo goes down or the versions we expect are removed this test may
# need an update.
#
# TODO:
# If this feature is to be supported and maintained long term, create a
# dedicated repo with known versions specifically for this test harness.
#
#
# ---------------------------------------------------------------------------- #

load test_support.bash;

# bats file_tags=search


# ---------------------------------------------------------------------------- #

setup_file() {
  skip "Skipping --bash-passthru tests";
  common_file_setup file;
  "$FLOX_CLI" subscribe netlify_test_ github:flox-examples/nixpkgs-netlify/main;
}


# ---------------------------------------------------------------------------- #

# Make sure we haven't broken regular search
@test "flox search hello" {
  run "$FLOX_CLI" search hello;
  assert_success;
}


# ---------------------------------------------------------------------------- #

# bats file_tags=search, semver

# ---------------------------------------------------------------------------- #

# Make sure we haven't broken regular search
@test "flox search -v hello" {
  run "$FLOX_CLI" search -v hello;
  assert_success;
}


# ---------------------------------------------------------------------------- #

# Make sure we haven't broken regular search
@test "flox search --json hello" {
  run bash -c "{ $FLOX_CLI search --json hello||:; }|jq;";
  assert_success;
}


# ---------------------------------------------------------------------------- #

@test "flox search node@18" {
  run "$FLOX_CLI" search node@18;
  assert_success;
  assert_output --partial 'netlify_test_.nodejs@18.10.0';
  assert_output --partial 'netlify_test_.nodejs@18.12.1';
  assert_output --partial 'netlify_test_.nodejs@18.13.0';
  assert_output --partial 'netlify_test_.nodejs@18.14.1';
  assert_output --partial 'netlify_test_.nodejs@18.14.2';
  refute_output --regexp 'netlify_test_\.nodejs@1[^8]\.';
}


# ---------------------------------------------------------------------------- #

@test "flox search 'node@^16.14'" {
  run "$FLOX_CLI" search 'node@^16.14';
  assert_success;
  refute_output --regexp 'netlify_test_\.nodejs@1[^6]\.';
  # This is not compatible with 16.14
  refute_output --partial 'netlify_test_.nodejs@16.13.2';
  # These are all `>= 16.14' so they should appear.
  assert_output --partial 'netlify_test_.nodejs@16.14.2'
  assert_output --partial 'netlify_test_.nodejs@16.15.0'
  assert_output --partial 'netlify_test_.nodejs@16.16.0'
}


# ---------------------------------------------------------------------------- #

@test "flox search 'node@^16.14 || 18.16.0'" {
  run "$FLOX_CLI" search 'node@^16.14 || 18.16.0';
  assert_success;
  # This is not compatible with 16.14
  refute_output --partial 'netlify_test_.nodejs@16.13.2';
  # Don't let other v18s appear
  refute_output --partial 'netlify_test_.nodejs@18.10.0';
  refute_output --partial 'netlify_test_.nodejs@18.12.1';
  refute_output --partial 'netlify_test_.nodejs@18.13.0';
  refute_output --partial 'netlify_test_.nodejs@18.14.1';
  refute_output --partial 'netlify_test_.nodejs@18.14.2';
  # These are all `>= 16.14' so they should appear.
  assert_output --partial 'netlify_test_.nodejs@16.14.2'
  assert_output --partial 'netlify_test_.nodejs@16.15.0'
  assert_output --partial 'netlify_test_.nodejs@16.16.0'
  # Make sure 18.16.0 appears
  assert_output --partial 'netlify_test_.nodejs@18.16.0'
}


# ---------------------------------------------------------------------------- #

# Make sure we emit valid JSON
@test "flox search 'node@^16.14' --json" {
  run bash -c "{ $FLOX_CLI search 'node@^16.14' --json||:; }|jq;";
  assert_success;
}


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
