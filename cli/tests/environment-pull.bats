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
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/project-push-${BATS_TEST_NUMBER?}"
  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" > /dev/null || return
}

project_teardown() {
  popd > /dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
}

# ---------------------------------------------------------------------------- #

setup() {
  home_setup test
  common_test_setup
  project_setup
  floxhub_setup "owner"
  make_dummy_env "owner" "name"
}
teardown() {
  unset _FLOX_FLOXHUB_GIT_URL
  project_teardown
  common_test_teardown
}

function make_dummy_env() {
  OWNER="$1"
  shift
  ENV_NAME="$1"
  shift

  pushd "$(mktemp -d)" > /dev/null || return
  "$FLOX_BIN" init --name "$ENV_NAME"
  "$FLOX_BIN" push --owner "$OWNER"
  "$FLOX_BIN" delete --force
  popd > /dev/null || return
}

# push an update to floxhub from another peer
function update_dummy_env() {
  OWNER="$1"
  shift
  ENV_NAME="$1"
  shift

  "$FLOX_BIN" install gzip --remote "$OWNER/$ENV_NAME"
}

# make the environment with specified owner and name incompatible with the current system
function make_incompatible() {
  OWNER="$1"
  shift
  ENV_NAME="$1"
  shift

  init_system=
  # replace linux with darwin or darwin with linux
  if [ -z "${NIX_SYSTEM##*-linux}" ]; then
    init_system="${NIX_SYSTEM%%-linux}-darwin"
  elif [ -z "${NIX_SYSTEM#*-darwin}" ]; then
    init_system="${NIX_SYSTEM%%-darwin}-linux"
  else
    echo "unknown system: '$NIX_SYSTEM'"
    exit 1
  fi


  git clone "$FLOX_FLOXHUB_PATH/$OWNER/floxmeta" "$PROJECT_DIR/floxmeta"
  pushd "$PROJECT_DIR/floxmeta" > /dev/null || return
  git checkout "$ENV_NAME"
  sed -i "s|$NIX_SYSTEM|$init_system|g" 2/env/manifest.toml 2/env/manifest.lock
  git add .
  git \
    -c "user.name=test" \
    -c "user.email=test@email.address" \
    commit \
    -m "make unsupported system"
  git push
  popd > /dev/null || return
  rm -rf "$PROJECT_DIR/floxmeta"
}

# ---------------------------------------------------------------------------- #
# bats test_tags=pull,pull:logged-out
@test "l1: pull login: running flox pull without login succeeds" {
  unset FLOX_FLOXHUB_TOKEN # logout, effectively

  run "$FLOX_BIN" pull --remote owner/name # dummy remote as we are not actually pulling anything
  assert_success
}

# bats test_tags=pull:l2,pull:l2:a,pull:l4
@test "l2.a/l4: flox pull accepts a floxhub namespace/environment, creates .flox if it does not exist" {
  run "$FLOX_BIN" pull --remote owner/name # dummy remote as we are not actually pulling anything
  assert_success
  assert [ -e ".flox/env.json" ]
  assert [ -e ".flox/env.lock" ]
  assert [ $(cat .flox/env.json | jq -r '.name') == "name" ]
  assert [ $(cat .flox/env.json | jq -r '.owner') == "owner" ]
}

# bats test_tags=pull:l2,pull:l2:b
@test "l2.b: flox pull with --remote fails if an env is already present" {

  "$FLOX_BIN" pull --remote owner/name # dummy remote as we are not actually pulling anything

  run "$FLOX_BIN" pull --remote owner/name # dummy remote as we are not actually pulling anything
  assert_failure

  # todo: error message
  # assert_output --partial <error message>
}

# bats test_tags=pull:l2,pull:l2:c
@test "l2.c: flox pull with --remote and --dir pulls into the specified directory" {

  run "$FLOX_BIN" pull --remote owner/name --dir ./inner
  assert_success
  assert [ -e "inner/.flox/env.json" ]
  assert [ -e "inner/.flox/env.lock" ]
  assert [ $(cat inner/.flox/env.json | jq -r '.name') == "name" ]
  assert [ $(cat inner/.flox/env.json | jq -r '.owner') == "owner" ]
}

# bats test_tags=pull:l3,pull:l3:a
@test "l3.a: pulling without namespace/environment" {

  "$FLOX_BIN" pull --remote owner/name # dummy remote as we are not actually pulling anything
  LOCKED_BEFORE=$(cat .flox/env.lock | jq -r '.rev')

  update_dummy_env "owner" "name"

  run "$FLOX_BIN" pull
  assert_success

  LOCKED_AFTER=$(cat .flox/env.lock | jq -r '.rev')

  assert [ "$LOCKED_BEFORE" != "$LOCKED_AFTER" ]
}

# bats test_tags=pull:l3,pull:l3:b
@test "l3.b: pulling without namespace/environment respects --dir" {

  "$FLOX_BIN" pull --remote owner/name --dir ./inner # dummy remote as we are not actually pulling anything
  LOCKED_BEFORE=$(cat ./inner/.flox/env.lock | jq -r '.rev')

  update_dummy_env "owner" "name"

  run "$FLOX_BIN" pull --dir ./inner
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

  "$FLOX_BIN" pull --remote owner/name --dir first
  LOCKED_FIRST_BEFORE=$(cat ./first/.flox/env.lock | jq -r '.rev')

  update_dummy_env "owner" "name"
  LOCKED_FIRST_AFTER=$(cat ./first/.flox/env.lock | jq -r '.rev')

  "$FLOX_BIN" pull --remote owner/name --dir second
  LOCKED_SECOND=$(cat ./second/.flox/env.lock | jq -r '.rev')

  assert [ "$LOCKED_FIRST_BEFORE" == "$LOCKED_FIRST_AFTER" ]
  assert [ "$LOCKED_FIRST_BEFORE" != "$LOCKED_SECOND" ]

  # after pulling first env, its at the rame rev as the second that was pulled after the update
  "$FLOX_BIN" pull --dir first

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
  skip "floxtest/default is not available for all systems"
  unset _FLOX_FLOXHUB_GIT_URL
  run "$FLOX_BIN" pull --remote floxtest/default
  assert_success
}

# ---------------------------------------------------------------------------- #

# bats test_tags=pull:unsupported
# pulling an environment without packages for the current platform
#should fail with an error
@test "pull environment without packages for the current platform fails" {
  update_dummy_env "owner" "name"
  make_incompatible "owner" "name"

  run "$FLOX_BIN" pull --remote owner/name
  assert_failure
  assert_output --partial "This environment is not yet compatible with your system ($NIX_SYSTEM)"
}

# bats test_tags=pull:add-system-flag
# pulling an environment without packages for the current platform
#should fail with an error
@test "pull environment without packages for the current platform succeeds with '--add-system' flag" {
  update_dummy_env "owner" "name"
  make_incompatible "owner" "name"

  run "$FLOX_BIN" pull --remote owner/name --add-system
  assert_success
}

# bats test_tags=pull:unsupported:prompt-fail
# pulling an environment without packages for the current platform
# should fail with an error
@test "pull environment without packages for the current platform prompts for about adding system" {
  update_dummy_env "owner" "name"
  make_incompatible "owner" "name"

  run -0 expect -d "$TESTS_DIR/pull/promptAmendSystem.exp" owner/name "$NIX_SYSTEM" no
  assert_success
  assert_output --partial "The environment you are trying to pull is not yet compatible with your system ($NIX_SYSTEM)"
  assert_line --partial "Did not pull the environment."

  assert [ ! -e ".flox/" ]
}

# bats test_tags=pull:unsupported:prompt-success
# pulling an environment without packages for the current platform
#should fail with an error
@test "pull environment without packages for the current platform prompts for about adding system: produces env" {
  update_dummy_env "owner" "name"
  make_incompatible "owner" "name"

  run -0 expect -d "$TESTS_DIR/pull/promptAmendSystem.exp" owner/name "$NIX_SYSTEM" yes
  assert_success

  run "$FLOX_BIN" list
  assert_success
}
