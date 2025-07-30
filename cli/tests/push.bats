#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test rust impl of `flox pull`
#
# ---------------------------------------------------------------------------- #

load test_support.bash
# bats file_tags=push

# ---------------------------------------------------------------------------- #

# Helpers for project based tests.

project_setup() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/project-push-${BATS_TEST_NUMBER?}"
  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" >/dev/null || return
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/custom/empty/resp.yaml"
}

project_teardown() {
  popd >/dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
}

# ---------------------------------------------------------------------------- #

setup() {
  common_test_setup
  setup_isolated_flox
  project_setup
  floxhub_setup "owner"
}
teardown() {
  project_teardown
  common_test_teardown
}

# simulate a dummy env update pushed to floxhub
function update_dummy_env() {
  OWNER="$1"
  shift
  ENV_NAME="$1"
  shift

  FLOXHUB_FLOXMETA_DIR="$BATS_TEST_TMPDIR/floxhub/$OWNER/floxmeta"

  pushd "$FLOXHUB_FLOXMETA_DIR" >/dev/null || return

  touch new_file
  git add .
  git commit -m "update"

  popd >/dev/null || return
}

# ---------------------------------------------------------------------------- #
# These don't need the catalog

# bats test_tags=push:h1
@test "h2: push login: running flox push before user has login metadata prompts the user to login" {
  unset FLOX_FLOXHUB_TOKEN # logout, effectively

  run "$FLOX_BIN" config

  run "$FLOX_BIN" init

  run "$FLOX_BIN" push --owner owner # dummy owner
  assert_failure
  assert_output --partial 'You are not logged in to FloxHub.'
}

# bats test_tags=push:h1:expired
@test "h2: push login: running flox with an expired token prompts the user to login" {
  # set an expired token
  export FLOX_FLOXHUB_TOKEN="eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.eyJodHRwczovL2Zsb3guZGV2L2hhbmRsZSI6InRlc3QiLCJleHAiOjE3MDQwNjM2MDB9.-5VCofPtmYQuvh21EV1nEJhTFV_URkRP0WFu4QDPFxY"

  run "$FLOX_BIN" init
  assert_output --partial 'Your FloxHub token has expired.'

  run "$FLOX_BIN" push --owner owner # dummy owner
  assert_failure
  assert_output --partial 'You are not logged in to FloxHub.'
}

# bats test_tags=push:broken
@test "push: broken: if you attempt to flox push an environment that fails to build then the push should fail with a message." {
  run "$FLOX_BIN" init

  init_system="$(get_system_other_than_current)"

  tomlq --in-place -t ".options.systems=[\"$init_system\"]" .flox/env/manifest.toml

  run "$FLOX_BIN" push --owner owner # dummy owner
  assert_failure
  assert_output --partial "Unable to push environment with build errors."
}

# bats test_tags=push:h3
@test "h2: push login: running flox push creates a remotely managed environment stored in the FloxHub" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.yaml"
  mkdir -p "machine_a"
  mkdir -p "machine_b"

  pushd "machine_a" >/dev/null || return
  "$FLOX_BIN" init --name "test"
  "$FLOX_BIN" install hello
  "$FLOX_BIN" push --owner owner
  popd >/dev/null || return

  pushd "machine_b" >/dev/null || return
  run "$FLOX_BIN" pull --remote owner/test
  assert_success

  run "$FLOX_BIN" list --name
  assert_success
  assert_line "hello"

  popd >/dev/null || return
}

# bats test_tags=push:h5
@test "h5: unique upstream environments: if you attempt to flox push an environment with the same name but different provenance from upstream then the push should fail with a message." {
  mkdir -p "machine_a"
  mkdir -p "machine_b"

  # Create an environment owner/test on machine_a and push it to floxhub
  pushd "machine_a" >/dev/null || return
  "$FLOX_BIN" init --name "test"
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/vim.yaml"
  "$FLOX_BIN" install vim

  # Also uses the `vim` resolution since it makes sure the manifest is locked before pushing
  "$FLOX_BIN" push --owner owner
  popd >/dev/null || return

  # Create an environment owner/test on machine_b and try to push it to floxhub
  # this should fail as an envrioment with the same name but different provenance already exists on floxhub
  pushd "machine_b" >/dev/null || return
  echo "trying to push to the same upstream env" >&3

  "$FLOX_BIN" init --name "test"
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/emacs.yaml"
  "$FLOX_BIN" install emacs

  # Also uses the `emacs` resolution since it makes sure the manifest is locked before pushing
  run "$FLOX_BIN" push --owner owner
  assert_failure
  assert_output --partial "An environment named owner/test already exists!"
  popd >/dev/null || return
}

# bats test_tags=push:h6
@test "h6: force push upstream: adding -f option to flox push will force push an environment upstream even if an existing environment of the same name exists with different provenance." {
  mkdir -p "machine_a" "machine_b" "machine_c"

  # Create an environment owner/test on machine_a and push it to floxhub
  pushd "machine_a" >/dev/null || return
  "$FLOX_BIN" init --name "test"
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/vim.yaml"
  "$FLOX_BIN" install vim
  # Also uses the `vim` resolution since it makes sure the manifest is locked before pushing
  "$FLOX_BIN" push --owner owner
  popd >/dev/null || return

  # Create an environment owner/test on machine_b and force-push it to floxhub
  pushd "machine_b" > /dev/null || return
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/custom/empty/resp.yaml"
  "$FLOX_BIN" init --name "test"
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/emacs.yaml"
  "$FLOX_BIN" install emacs
  # Also uses the `emacs` resolution since it makes sure the manifest is locked before pushing
  run "$FLOX_BIN" push --owner owner --force
  assert_success
  popd >/dev/null || return

  # Pull the environment owner/test on machine_c and check that it has the emacs package
  pushd "machine_c" > /dev/null || return
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/custom/empty/resp.yaml"
  "$FLOX_BIN" pull --remote owner/test
  run "$FLOX_BIN" list --name
  assert_success
  assert_line "emacs"
  popd >/dev/null || return
}

# bats test_tags=push:composed:local-include
@test "push composed environment: fail if includes local env" {

  mkdir -p composer included
  "$FLOX_BIN" init --dir composer

  # create and lock included env
  "$FLOX_BIN" init --dir included
  "$FLOX_BIN" list --dir included -c  | "$FLOX_BIN" edit --dir included -f -

  # include local env
  "$FLOX_BIN" edit -d composer -f - <<EOF
  version = 1
  include.environments = [{ dir = "../included" }]
EOF

  run "$FLOX_BIN" push --dir composer --owner owner
  assert_failure
  assert_line "❌ ERROR: cannot push environment that includes local environments"
}


# bats test_tags=push:composed:local-include
@test "push composed environment (managed): fail if includes local env" {
  mkdir -p composer included
  "$FLOX_BIN" init --dir composer
  # make composer managed
  "$FLOX_BIN" push --dir composer --owner owner

  # create and lock included env
  "$FLOX_BIN" init --dir included
  "$FLOX_BIN" list --dir included -c  | "$FLOX_BIN" edit --dir included -f -

  # include local env
  "$FLOX_BIN" edit -d composer -f - <<EOF
  version = 1
  include.environments = [{ dir = "../included" }]
EOF

  run "$FLOX_BIN" push --dir composer --owner owner
  assert_failure
  assert_line "❌ ERROR: cannot push environment that includes local environments"
}
