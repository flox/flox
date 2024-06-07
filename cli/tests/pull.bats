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
  make_dummy_env "owner" "name"

  export FLOX_FEATURES_USE_CATALOG=true
  export _FLOX_USE_CATALOG_MOCK="$TESTS_DIR/catalog_responses/empty_responses.json"

  export UNSUPPORTED_SYSTEM_PROMPT="The environment you are trying to pull is not yet compatible with your system ($NIX_SYSTEM)."
  export UNSUPPORTED_PACKAGE_PROMPT="The environment you are trying to pull could not be built locally."
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

  _FLOX_USE_CATALOG_MOCK="$TESTS_DIR/catalog_responses/resolve/gzip.json" \
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
    package='["darwin", "ps"]'
  elif [ -z "${NIX_SYSTEM#*-darwin}" ]; then
    package='["glibc"]'
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

# make the environment with specified owner and name incompatible with the current system
# by adding a package that fails nix evaluation due to being on an unsupported system.
function add_insecure_package() {
  OWNER="$1"
  shift
  ENV_NAME="$1"
  shift

  git clone "$FLOX_FLOXHUB_PATH/$OWNER/floxmeta" "$PROJECT_DIR/floxmeta"
  pushd "$PROJECT_DIR/floxmeta" >/dev/null || return
  git checkout "$ENV_NAME"
  tomlq --in-place --toml-output '.install.extra."pkg-path" = ["python2"]' 2/env/manifest.toml
  git add .
  git \
    -c "user.name=test" \
    -c "user.email=test@email.address" \
    commit \
    -m "add failing package"
  git push
  popd >/dev/null || return
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
  export FLOX_FEATURES_USE_CATALOG=false

  run "$FLOX_BIN" pull --remote owner/name # dummy remote as we are not actually pulling anything
  assert_success
  assert [ -e ".flox/env.json" ]
  assert [ -e ".flox/env.lock" ]
  assert [ $(cat .flox/env.json | jq -r '.name') == "name" ]
  assert [ $(cat .flox/env.json | jq -r '.owner') == "owner" ]
}

# bats test_tags=pull:l2,pull:l2:b
@test "l2.b: flox pull with --remote fails if an env is already present" {
  export FLOX_FEATURES_USE_CATALOG=false

  "$FLOX_BIN" pull --remote owner/name # dummy remote as we are not actually pulling anything

  run "$FLOX_BIN" pull --remote owner/name # dummy remote as we are not actually pulling anything
  assert_failure

  # todo: error message
  # assert_output --partial <error message>
}

# bats test_tags=pull:l2,pull:l2:c
@test "l2.c: flox pull with --remote and --dir pulls into the specified directory" {
  export FLOX_FEATURES_USE_CATALOG=false

  run "$FLOX_BIN" pull --remote owner/name --dir ./inner
  assert_success
  assert [ -e "inner/.flox/env.json" ]
  assert [ -e "inner/.flox/env.lock" ]
  assert [ $(cat inner/.flox/env.json | jq -r '.name') == "name" ]
  assert [ $(cat inner/.flox/env.json | jq -r '.owner') == "owner" ]
}

# bats test_tags=pull:l3,pull:l3:a
@test "l3.a: pulling without namespace/environment" {
  export FLOX_FEATURES_USE_CATALOG=false

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
  export FLOX_FEATURES_USE_CATALOG=false

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

  export FLOX_FEATURES_USE_CATALOG=false

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

# bats test_tags=pull:floxhub
# try pulling from floxhub authenticated with a test token
@test "l?: pull environment from FloxHub" {
  skip "floxtest/default is not available for all systems"
  unset _FLOX_FLOXHUB_GIT_URL
  run "$FLOX_BIN" pull --remote floxtest/default
  assert_success
}

# ---------------------------------------------------------------------------- #

# bats test_tags=pull:add-system-flag
# pulling an environment without packages for the current platform
#should fail with an error
@test "pull environment inside the same environment without the '--force' flag" {
  export FLOX_FEATURES_USE_CATALOG=false

  update_dummy_env "owner" "name"

  run "$FLOX_BIN" pull --remote owner/name
  assert_success
  run "$FLOX_BIN" pull --remote owner/name
  assert_failure
}

# bats test_tags=pull:add-system-flag
# pulling an environment without packages for the current platform
@test "pull environment inside the same environment with '--force' flag" {
  export FLOX_FEATURES_USE_CATALOG=false

  update_dummy_env "owner" "name"

  run "$FLOX_BIN" pull --remote owner/name
  assert_success
  run "$FLOX_BIN" pull --remote owner/name --force
  assert_success
}

# bats test_tags=pull:unsupported:warning
# An environment that is not compatible with the current ssystem
# due to the current system missing <system> in `option.systems`
# AND a package that is indeed not able to be built for the current system
# should show a warning, but otherwise succeed to pull
@test "pull unsupported environment succeeds with '--force' flag but shows warning if unable to build still" {
  export FLOX_FEATURES_USE_CATALOG=false

  update_dummy_env "owner" "name"

  make_incompatible "owner" "name"
  add_incompatible_package "owner" "name"

  run "$FLOX_BIN" pull --remote owner/name --force
  assert_success
  assert_line --partial "Modified the manifest to include your system but could not build."

  run "$FLOX_BIN" list
  assert_success
}

# ---------------------------------------------------------------------------- #

# bats test_tags=activate:remote:incompatible
# activating an incompatible environment should fail gracefully
@test "activate incompatible environment fails gracefully" {
  export FLOX_FEATURES_USE_CATALOG=false

  update_dummy_env "owner" "name"
  make_incompatible "owner" "name"

  run "$FLOX_BIN" activate --remote owner/name --trust
  assert_failure
  assert_output --partial "This environment is not yet compatible with your system ($NIX_SYSTEM)"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=pull:unsupported-package
# pulling an environment with a package that is not available for the current platform
# should fail with an error
@test "pull environment with package not available for the current platform fails" {
  export FLOX_FEATURES_USE_CATALOG=false

  update_dummy_env "owner" "name"
  add_incompatible_package "owner" "name"

  run "$FLOX_BIN" pull --remote owner/name

  assert_failure
  assert_line --partial "package 'extra' is not available for this system ('$NIX_SYSTEM')"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=pull:eval-failure
# pulling an environment with a package that fails to evaluate
# should fail with an error
@test "pull environment with insecure package fails to evaluate" {
  export FLOX_FEATURES_USE_CATALOG=false

  update_dummy_env "owner" "name"
  add_insecure_package "owner" "name"

  run "$FLOX_BIN" pull --remote owner/name

  assert_failure
  assert_line --partial "package 'extra' failed to evaluate:"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=pull:up-to-date
# updating an up-to-date environment should return with an info message
@test "pull up-to-date env returns info message" {
  export FLOX_FEATURES_USE_CATALOG=false

  # pull a fresh environment
  "$FLOX_BIN" pull --remote owner/name
  # pull it again, and expect an info message
  run "$FLOX_BIN" pull
  assert_success
  assert_line --partial "already up to date."
}

# ----------------------------- Catalog Tests -------------------------------- #
# ---------------------------------------------------------------------------- #

# bats test_tags=pull:l2,pull:l2:a,pull:l4
@test "catalog: l2.a/l4: flox pull accepts a floxhub namespace/environment, creates .flox if it does not exist" {

  # dummy environment has no packages to resolve
  export _FLOX_USE_CATALOG_MOCK="$TESTS_DIR/catalog_responses/empty_responses.json"

  # dummy remote as we are not actually pulling anything
  run "$FLOX_BIN" pull --remote owner/name
  assert_success
  assert [ -e ".flox/env.json" ]
  assert [ -e ".flox/env.lock" ]
  assert [ $(cat .flox/env.json | jq -r '.name') == "name" ]
  assert [ $(cat .flox/env.json | jq -r '.owner') == "owner" ]
}

# bats test_tags=pull:l2,pull:l2:b
@test "catalog: l2.b: flox pull with --remote fails if an env is already present" {
  # dummy environment has no packages to resolve
  export _FLOX_USE_CATALOG_MOCK="$TESTS_DIR/catalog_responses/empty_responses.json"

  "$FLOX_BIN" pull --remote owner/name # dummy remote as we are not actually pulling anything

  run "$FLOX_BIN" pull --remote owner/name # dummy remote as we are not actually pulling anything
  assert_failure
}

# bats test_tags=pull:l2,pull:l2:c
@test "catalog: l2.c: flox pull with --remote and --dir pulls into the specified directory" {
  # dummy environment has no packages to resolve
  export _FLOX_USE_CATALOG_MOCK="$TESTS_DIR/catalog_responses/empty_responses.json"

  run "$FLOX_BIN" pull --remote owner/name --dir ./inner
  assert_success
  assert [ -e "inner/.flox/env.json" ]
  assert [ -e "inner/.flox/env.lock" ]
  assert [ $(cat inner/.flox/env.json | jq -r '.name') == "name" ]
  assert [ $(cat inner/.flox/env.json | jq -r '.owner') == "owner" ]
}

# ---------------------------------------------------------------------------- #

# ---------------------------------------------------------------------------- #

# ---------------------------------------------------------------------------- #

# ---------------------------------------------------------------------------- #

# ---------------------------------------------------------------------------- #
