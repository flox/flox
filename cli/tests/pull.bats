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
  pushd "$PROJECT_DIR" >/dev/null || return
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

  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.json"

  export UNSUPPORTED_SYSTEM_PROMPT="The environment you are trying to pull is not yet compatible with your system ($NIX_SYSTEM)."
  export UNSUPPORTED_PACKAGE_PROMPT="The environment you are trying to pull could not be built locally."
}

teardown() {
  unset _FLOX_FLOXHUB_GIT_URL
  wait_for_watchdogs "$PROJECT_DIR" || return 1
  project_teardown
  common_test_teardown
}

function make_dummy_env() {
  OWNER="$1"
  shift
  ENV_NAME="$1"
  shift

  pushd "$(mktemp -d)" >/dev/null || return
  "$FLOX_BIN" init --name "$ENV_NAME"
  "$FLOX_BIN" push --owner "$OWNER"
  "$FLOX_BIN" delete --force
  popd >/dev/null || return
}

# push an update to floxhub from another peer
function update_dummy_env() {
  OWNER="$1"
  shift
  ENV_NAME="$1"
  shift

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/gzip.json" \
    "$FLOX_BIN" install gzip --remote "$OWNER/$ENV_NAME"
}

# make the environment with specified owner and name incompatible with the current system
# by changing setting `option.systems = [<not the current system>]`
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
  pushd "$PROJECT_DIR/floxmeta" >/dev/null || return
  git checkout "$ENV_NAME"
  sed -i "s|$NIX_SYSTEM|$init_system|g" 2/env/manifest.toml 2/env/manifest.lock

  git add .
  git \
    -c "user.name=test" \
    -c "user.email=test@email.address" \
    commit \
    -m "make unsupported system"
  git push
  popd >/dev/null || return
  rm -rf "$PROJECT_DIR/floxmeta"
}

function copy_manifest_and_lockfile_to_remote() {
  OWNER="$1"
  shift
  ENV_NAME="$1"
  shift
  ENV_FILES_DIR="$1"
  shift

  git clone "$FLOX_FLOXHUB_PATH/$OWNER/floxmeta" "$PROJECT_DIR/floxmeta"
  pushd "$PROJECT_DIR/floxmeta" >/dev/null || return
  git checkout "$ENV_NAME"
  cp "$ENV_FILES_DIR/manifest.toml" 2/env/manifest.toml
  cp "$ENV_FILES_DIR/manifest.lock" 2/env/manifest.lock

  git add .
  git \
    -c "user.name=test" \
    -c "user.email=test@email.address" \
    commit \
    -m "copy manifest and lockfile"
  git push
  popd >/dev/null || return
  rm -rf "$PROJECT_DIR/floxmeta"
}

# catalog manifests by default support all systems.
# remove additional systems to check handling of missing systems.
# should be run on an empty environment.
function remove_extra_systems() {
  OWNER="$1"
  shift
  ENV_NAME="$1"
  shift

  git clone "$FLOX_FLOXHUB_PATH/$OWNER/floxmeta" "$PROJECT_DIR/floxmeta"
  pushd "$PROJECT_DIR/floxmeta" >/dev/null || return
  git checkout "$ENV_NAME"

  tomlq --in-place --toml-output ".options.systems = [\"$NIX_SYSTEM\"]" 1/env/manifest.toml

  git add .
  git \
    -c "user.name=test" \
    -c "user.email=test@email.address" \
    commit \
    -m "remove extra systems"
  git push
  popd >/dev/null || return
  rm -rf "$PROJECT_DIR/floxmeta"
}

# make the environment with specified owner and name incompatible with the current system
# by adding a package that fails nix evaluation due to being on an unsupported system.
function add_incompatible_package() {
  OWNER="$1"
  shift
  ENV_NAME="$1"
  shift

  package=
  # replace linux with darwin or darwin with linux
  if [ -z "${NIX_SYSTEM##*-linux}" ]; then
    package='"darwin.ps"'
    export INCOMPATIBLE_MOCK_RESPONSE="$GENERATED_DATA/resolve/darwin_ps_incompatible.json"

  elif [ -z "${NIX_SYSTEM#*-darwin}" ]; then
    package='"glibc"'
    export INCOMPATIBLE_MOCK_RESPONSE="$GENERATED_DATA/resolve/glibc_incompatible.json"
  else
    echo "unknown system: '$NIX_SYSTEM'"
    exit 1
  fi

  git clone "$FLOX_FLOXHUB_PATH/$OWNER/floxmeta" "$PROJECT_DIR/floxmeta"
  pushd "$PROJECT_DIR/floxmeta" >/dev/null || return
  git checkout "$ENV_NAME"
  tomlq --in-place --toml-output ".install.extra.\"pkg-path\" = $package" 2/env/manifest.toml

  git add .
  git \
    -c "user.name=test" \
    -c "user.email=test@email.address" \
    commit \
    -m "make unsupported system"
  git push
  popd >/dev/null || return
  rm -rf "$PROJECT_DIR/floxmeta"
}

# ---------------------------------------------------------------------------- #
# bats test_tags=pull,pull:logged-out
@test "l1: pull login: running flox pull without login succeeds" {
  make_dummy_env "owner" "name"
  unset FLOX_FLOXHUB_TOKEN # logout, effectively

  run "$FLOX_BIN" pull --remote owner/name # dummy remote as we are not actually pulling anything
  assert_success
}

# bats test_tags=pull:floxhub
# try pulling from floxhub authenticated with a test token
@test "l?: pull environment from FloxHub" {
  skip "floxtest/default is not available for all systems"
  unset _FLOX_FLOXHUB_GIT_URL
  run "$FLOX_BIN" pull --remote floxtest/default
  assert_success
}

# ---------------------------------------------------------------------------- #

# bats test_tags=pull:up-to-date
# updating an up-to-date environment should return with an info message
@test "pull up-to-date env returns info message" {
  make_dummy_env "owner" "name"

  # dummy environment has no packages to resolve
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.json"

  # pull a fresh environment
  "$FLOX_BIN" pull --remote owner/name
  # pull it again, and expect an info message
  run "$FLOX_BIN" pull
  assert_success
  assert_line --partial "already up to date."
}

# bats test_tags=pull:l2,pull:l2:a,pull:l4
@test "l2.a/l4: flox pull accepts a floxhub namespace/environment, creates .flox if it does not exist" {
  make_dummy_env "owner" "name"

  # dummy environment has no packages to resolve
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.json"

  # dummy remote as we are not actually pulling anything
  run "$FLOX_BIN" pull --remote owner/name
  assert_success
  assert [ -e ".flox/env.json" ]
  assert [ -e ".flox/env.lock" ]
  assert [ $(cat .flox/env.json | jq -r '.name') == "name" ]
  assert [ $(cat .flox/env.json | jq -r '.owner') == "owner" ]
}

# bats test_tags=pull:l2,pull:l2:b
@test "l2.b: flox pull with --remote fails if an env is already present" {
  make_dummy_env "owner" "name"

  # dummy environment has no packages to resolve
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.json"

  "$FLOX_BIN" pull --remote owner/name # dummy remote as we are not actually pulling anything

  run "$FLOX_BIN" pull --remote owner/name # dummy remote as we are not actually pulling anything
  assert_failure
}

# bats test_tags=pull:l2,pull:l2:c
@test "l2.c: flox pull with --remote and --dir pulls into the specified directory" {
  make_dummy_env "owner" "name"

  # dummy environment has no packages to resolve
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.json"

  run "$FLOX_BIN" pull --remote owner/name --dir ./inner
  assert_success
  assert [ -e "inner/.flox/env.json" ]
  assert [ -e "inner/.flox/env.lock" ]
  assert [ $(cat inner/.flox/env.json | jq -r '.name') == "name" ]
  assert [ $(cat inner/.flox/env.json | jq -r '.owner') == "owner" ]
}

# bats test_tags=pull:l3,pull:l3:a
@test "l3.a: pulling without namespace/environment" {
  make_dummy_env "owner" "name"

  # dummy environment has no packages to resolve
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.json"

  "$FLOX_BIN" pull --remote owner/name # dummy remote as we are not actually pulling anything
  LOCKED_BEFORE=$(cat .flox/env.lock | jq -r '.rev')

  update_dummy_env "owner" "name"

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/gzip.json" \
    run "$FLOX_BIN" pull

  assert_success

  LOCKED_AFTER=$(cat .flox/env.lock | jq -r '.rev')

  assert [ "$LOCKED_BEFORE" != "$LOCKED_AFTER" ]
}

# bats test_tags=pull:l3,pull:l3:b
@test "l3.b: pulling without namespace/environment respects --dir" {
  make_dummy_env "owner" "name"

  # dummy environment has no packages to resolve
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.json"

  "$FLOX_BIN" pull --remote owner/name --dir ./inner # dummy remote as we are not actually pulling anything
  LOCKED_BEFORE=$(cat ./inner/.flox/env.lock | jq -r '.rev')

  update_dummy_env "owner" "name"

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/gzip.json" \
    run "$FLOX_BIN" pull --dir ./inner
  assert_success

  LOCKED_AFTER=$(cat ./inner/.flox/env.lock | jq -r '.rev')

  assert [ "$LOCKED_BEFORE" != "$LOCKED_AFTER" ]
}

# bats test_tags=pull:l6,pull:l6:a
@test "l6.a: pulling the same remote environment in multiple directories creates unique copies of the environment" {
  make_dummy_env "owner" "name"

  mkdir first second

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.json" \
    "$FLOX_BIN" pull --remote owner/name --dir first
  LOCKED_FIRST_BEFORE=$(cat ./first/.flox/env.lock | jq -r '.rev')

  update_dummy_env "owner" "name"
  LOCKED_FIRST_AFTER=$(cat ./first/.flox/env.lock | jq -r '.rev')

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.json" \
    "$FLOX_BIN" pull --remote owner/name --dir second
  LOCKED_SECOND=$(cat ./second/.flox/env.lock | jq -r '.rev')

  assert [ "$LOCKED_FIRST_BEFORE" == "$LOCKED_FIRST_AFTER" ]
  assert [ "$LOCKED_FIRST_BEFORE" != "$LOCKED_SECOND" ]

  # after pulling first env, its at the rame rev as the second that was pulled after the update
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/gzip.json" \
    "$FLOX_BIN" pull --dir first

  LOCKED_FIRST_AFTER_PULL=$(cat ./first/.flox/env.lock | jq -r '.rev')

  assert [ "$LOCKED_FIRST_BEFORE" != "$LOCKED_FIRST_AFTER_PULL" ]
  assert [ "$LOCKED_FIRST_AFTER_PULL" == "$LOCKED_SECOND" ]
}

# ---------------------------------------------------------------------------- #

# bats test_tags=pull:twice:no-force
@test "pull environment inside the same environment without the '--force' flag" {
  make_dummy_env "owner" "name"
  update_dummy_env "owner" "name"

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/gzip.json" \
    run "$FLOX_BIN" pull --remote owner/name
  assert_success

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/gzip.json" \
    run "$FLOX_BIN" pull --remote owner/name
  assert_failure
}

# bats test_tags=pull:twice:force
@test "pull environment inside the same environment with '--force' flag" {
  make_dummy_env "owner" "name"
  update_dummy_env "owner" "name"

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/gzip.json" \
    run "$FLOX_BIN" pull --remote owner/name
  assert_success

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/gzip.json" \
    run "$FLOX_BIN" pull --remote owner/name --force
  assert_success
}

# ---------------------------------------------------------------------------- #

# bats test_tags=pull:catalog:unsupported:warning
# An environment that is not compatible with the current system
# due to the current system missing <system> in `option.systems`
# AND a package that is indeed not able to be built for the current system
# should show a warning, but otherwise succeed to pull
@test "pull unsupported environment succeeds with '--force' flag but shows warning if unable to build still" {
  make_dummy_env "owner" "name"
  remove_extra_systems "owner" "name"
  update_dummy_env "owner" "name"
  make_incompatible "owner" "name"
  add_incompatible_package "owner" "name"

  # add_incompatible_package does not _lock_ the environment,
  # but pull won't either because it will expect it to already have a lock
  run "$FLOX_BIN" pull --remote owner/name
  assert_failure
  assert_line --partial "This environment is not yet compatible with your system ($NIX_SYSTEM)"

  _FLOX_USE_CATALOG_MOCK="$INCOMPATIBLE_MOCK_RESPONSE" \
    run "$FLOX_BIN" pull --remote owner/name --force
  assert_success
  assert_line --partial "Modified the manifest to include your system but could not build."

  run "$FLOX_BIN" list
  assert_success
}

# ---------------------------------------------------------------------------- #

# bats test_tags=pull:copy:new
@test "'pull --copy' creates path environment" {
  make_dummy_env "owner" "name"

  run "$FLOX_BIN" pull --remote owner/name --copy
  assert_success
  assert [ ! -e ".flox/env.lock" ]
  assert [ $(cat .flox/env.json | jq -r '.name') == "name" ]
  assert [ $(cat .flox/env.json | jq -r '.owner') == "null" ]
  assert_output --partial "Created path environment from owner/name"
}

# bats test_tags=pull:copy:new:error-if-incompatible
@test "'pull --copy' has same error semantics as normal 'pull'" {
  make_dummy_env "owner" "name"
  update_dummy_env "owner" "name"
  make_incompatible "owner" "name"

  run "$FLOX_BIN" pull --remote owner/name
  assert_failure
}

# bats test_tags=pull:copy:convert
@test "'pull --copy' converts managed env to path environment" {
  make_dummy_env "owner" "name"

  run "$FLOX_BIN" pull --remote owner/name
  assert_success
  assert [ -e ".flox/env.lock" ]
  assert [ $(cat .flox/env.json | jq -r '.name') == "name" ]
  assert [ $(cat .flox/env.json | jq -r '.owner') == "owner" ]

  run "$FLOX_BIN" pull --copy
  assert_success
  assert [ ! -e ".flox/env.lock" ]
  assert [ $(cat .flox/env.json | jq -r '.name') == "name" ]
  assert [ $(cat .flox/env.json | jq -r '.owner') == "null" ]
  assert_output --partial "Created path environment from owner/name"
}

# `flox pull --copy` is the recommended way to push an environmentto a new name
# if the original was deleted from FloxHub.
# bats test_tags=pull:copy:convert-if-deleted
@test "'pull --copy' converts to path environment even if upstream deleted" {
  make_dummy_env "owner" "name"

  "$FLOX_BIN" pull --remote owner/name

  # delete upstream
  rm -rf "$FLOX_FLOXHUB_PATH"

  run "$FLOX_BIN" pull --copy
  assert_success
}

# `flox pull --copy` should not update an existing environment
# bats test_tags=pull:copy:do-not-update-local
@test "'pull --copy' does not pull" {
  make_dummy_env "owner" "name"

  "$FLOX_BIN" pull --remote owner/name

  update_dummy_env "owner" "name"

  "$FLOX_BIN" pull --copy

  run "$FLOX_BIN" list
  assert_success
  assert_output --partial "No packages are installed"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=activate:remote:incompatible
# activating an incompatible environment should fail gracefully
@test "activate incompatible environment fails gracefully" {

  make_dummy_env "owner" "name"
  remove_extra_systems "owner" "name"
  update_dummy_env "owner" "name"
  make_incompatible "owner" "name"

  run "$FLOX_BIN" activate --remote owner/name --trust
  assert_failure
  assert_output --partial "This environment is not yet compatible with your system ($NIX_SYSTEM)"
}

# ---------------------------------------------------------------------------- #
