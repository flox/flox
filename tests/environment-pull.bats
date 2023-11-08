#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test rust impl of `flox pull`
#
# ---------------------------------------------------------------------------- #

load test_support.bash
# bats file_tags=pull

# ---------------------------------------------------------------------------- #

# Helpers for project based tests.

project_setup() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/test"
  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" >/dev/null || return
}

project_teardown() {
  popd >/dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
}

# ---------------------------------------------------------------------------- #

setup() {
  home_setup test;
  common_test_setup
  project_setup

  export FLOX_FLOXHUB_TOKEN=flox_testOAuthToken
  export __FLOX_FLOXHUB_URL="file://$BATS_TEST_TMPDIR/floxhub"

  make_dummy_env "owner" "name"
}
teardown() {
  unset __FLOX_FLOXHUB_URL
  project_teardown
  common_test_teardown
}

function make_dummy_env() {
  OWNER="$1"; shift;
  ENV_NAME="$1"; shift;

  FLOXHUB_FLOXMETA_DIR="$BATS_TEST_TMPDIR/floxhub/$OWNER/floxmeta"

  mkdir -p "$FLOXHUB_FLOXMETA_DIR"
  pushd "$FLOXHUB_FLOXMETA_DIR"

  # todo: fake a real upstream env
  git init --initial-branch="$NIX_SYSTEM.$ENV_NAME"

  git config user.name "test"
  git config user.email "test@email.address"

  touch "env.json"
  git add .
  git commit -m "initial commit"

  popd >/dev/null || return
}

# simulate a dummy env update pushed to floxhub
function update_dummy_env() {
  OWNER="$1"; shift;
  ENV_NAME="$1"; shift;

  FLOXHUB_FLOXMETA_DIR="$BATS_TEST_TMPDIR/floxhub/$OWNER/floxmeta"

  pushd "$FLOXHUB_FLOXMETA_DIR" >/dev/null || return

  touch new_file
  git add .
  git commit -m "update"

  popd >/dev/null || return
}

# ---------------------------------------------------------------------------- #


# bats test_tags=pull:l1
@test "l1: pull login: running flox pull before user has login metadata prompts the user to login" {

  unset FLOX_FLOXHUB_TOKEN; # logout, effectively

  run "$FLOX_CLI" pull --remote owner/name # dummy remote as we are not actually pulling anything
  assert_failure
  assert_output --partial 'Please login to floxhub with `flox auth login`'
}

# bats test_tags=pull:l2,pull:l2:a,pull:l4
@test "l2.a/l4: flox pull accepts a flox hub namespace/environment, creates .flox if it does not exist" {
  run "$FLOX_CLI" pull --remote owner/name # dummy remote as we are not actually pulling anything
  assert_success
  assert [ -e ".flox/env.json" ];
  assert [ -e ".flox/env.lock" ];
  assert [ $(cat .flox/env.json | jq -r '.name') == "name" ];
  assert [ $(cat .flox/env.json | jq -r '.owner') == "owner" ];
}

# bats test_tags=pull:l2,pull:l2:b
@test "l2.b: flox pull with --remote fails if an env is already present" {

  "$FLOX_CLI" pull --remote owner/name # dummy remote as we are not actually pulling anything

  run "$FLOX_CLI" pull --remote owner/name # dummy remote as we are not actually pulling anything
  assert_failure

  # todo: error message
  # assert_output --partial <error message>
}

# bats test_tags=pull:l2,pull:l2:c
@test "l2.c: flox pull with --remote and --dir pulls into the specified directory" {

  run "$FLOX_CLI" pull --remote owner/name --dir ./inner
  assert_success
  assert [ -e "inner/.flox/env.json" ];
  assert [ -e "inner/.flox/env.lock" ];
  assert [ $(cat inner/.flox/env.json | jq -r '.name') == "name" ];
  assert [ $(cat inner/.flox/env.json | jq -r '.owner') == "owner" ];
}


# bats test_tags=pull:l3,pull:l3:a
@test "l3.a: pulling without namespace/environment" {

  "$FLOX_CLI" pull --remote owner/name # dummy remote as we are not actually pulling anything
  LOCKED_BEFORE=$(cat .flox/env.lock | jq -r '.rev')

  update_dummy_env "owner" "name"

  run "$FLOX_CLI" pull
  assert_success

  LOCKED_AFTER=$(cat .flox/env.lock | jq -r '.rev')

  assert [ "$LOCKED_BEFORE" != "$LOCKED_AFTER" ]
}

# bats test_tags=pull:l3,pull:l3:b
@test "l3.b: pulling without namespace/environment respects --dir" {

  "$FLOX_CLI" pull --remote owner/name --dir ./inner # dummy remote as we are not actually pulling anything
  LOCKED_BEFORE=$(cat ./inner/.flox/env.lock | jq -r '.rev')

  update_dummy_env "owner" "name"

  run "$FLOX_CLI" pull --dir ./inner
  assert_success

  LOCKED_AFTER=$(cat ./inner/.flox/env.lock | jq -r '.rev')

  assert [ "$LOCKED_BEFORE" != "$LOCKED_AFTER" ]
}

#
# Notice: l5 is tested in l2.a and l2.c
#

# bats test_tags=pull:l6,pull:l6:a
@test "l6.a: pulling the same remote environment in multiple directories creates unique copies of the environment" {

  mkdir first second

  "$FLOX_CLI" pull --remote owner/name --dir first  # dummy remote as we are not actually pulling anything
  LOCKED_FIRST_BEFORE=$(cat ./first/.flox/env.lock | jq -r '.rev')

  update_dummy_env "owner" "name"
  LOCKED_FIRST_AFTER=$(cat ./first/.flox/env.lock | jq -r '.rev')

  "$FLOX_CLI" pull --remote owner/name --dir second  # dummy remote as we are not actually pulling anything
  LOCKED_SECOND=$(cat ./second/.flox/env.lock | jq -r '.rev')

  assert [ "$LOCKED_FIRST_BEFORE" == "$LOCKED_FIRST_AFTER" ]
  assert [ "$LOCKED_FIRST_BEFORE" != "$LOCKED_SECOND" ]

  # after pulling first env, its at the rame rev as the second that was pulled after the update
  "$FLOX_CLI" pull --dir first  # dummy remote as we are not actually pulling anything

  LOCKED_FIRST_AFTER_PULL=$(cat ./first/.flox/env.lock | jq -r '.rev')

  assert [ "$LOCKED_FIRST_BEFORE" != "$LOCKED_FIRST_AFTER_PULL" ]
  assert [ "$LOCKED_FIRST_AFTER_PULL" == "$LOCKED_SECOND" ]
}

# bats test_tags=pull:l6,pull:l6:b
@test "l6.b: installing in one directory doesn't show in the other until it is pushed and pulled again" {
  skip "pulling is not yet implemtened"
}

# bats test_tags=pull:floxhub
# try pulling from floxhub authenticated with a test token
@test "l?: pull environment from floxhub" {
  unset __FLOX_FLOXHUB_URL;
  run "$FLOX_CLI" pull --remote floxtest/default
  assert_success
}
