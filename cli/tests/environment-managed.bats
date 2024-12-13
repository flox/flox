#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test the managed environment feature of flox.
# * Tests whether "flox" commands work as expected in a managed environment
# * Tests conversion of a local environments to managed environments
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# ---------------------------------------------------------------------------- #

# Helpers for project based tests.

project_setup() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/project-managed-${BATS_TEST_NUMBER?}"
  export PROJECT_NAME="${PROJECT_DIR##*/}"
  export OWNER="owner"

  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" > /dev/null || return

}

project_teardown() {
  popd > /dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
}

setup() {
  common_test_setup
  setup_isolated_flox
  project_setup
  floxhub_setup "$OWNER"
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.json"
}

teardown() {
  cat_teardown_fifo
  wait_for_watchdogs "$PROJECT_DIR" || return 1
  project_teardown
  common_test_teardown
}

# ---------------------------------------------------------------------------- #

# init path environment and push to remote
function make_empty_remote_env() {
  "$FLOX_BIN" init
  "$FLOX_BIN" push --owner "$OWNER"
}

# ---------------------------------------------------------------------------- #

dot_flox_exists() {
  # Since the return value is based on the exit code of `test`:
  # 0 = true
  # 1 = false
  [[ -d "$PROJECT_DIR/.flox" ]]
}

# ---------------------------------------------------------------------------- #

# bats test_tags=install,managed
@test "m1: install a package to a managed environment" {
  make_empty_remote_env

  run --separate-stderr "$FLOX_BIN" list --name
  assert_success
  assert_output ""

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json" \
    run "$FLOX_BIN" install hello
  assert_success
  assert_output --partial "environment '$OWNER/project-managed-${BATS_TEST_NUMBER}'" # managed env output

  run --separate-stderr "$FLOX_BIN" list --name
  assert_success
  assert_output "hello"
}

# bats test_tags=uninstall,managed
@test "m2: uninstall a package from a managed environment" {
  make_empty_remote_env
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json" \
    "$FLOX_BIN" install hello

  run "$FLOX_BIN" uninstall hello
  assert_success

  run --separate-stderr "$FLOX_BIN" list --name
  assert_success
  assert_output ""
}

# bats test_tags=edit,managed
@test "m3: edit a package from a managed environment" {
  make_empty_remote_env

  TMP_MANIFEST_PATH="$BATS_TEST_TMPDIR/manifest.toml"

  cat << "EOF" >> "$TMP_MANIFEST_PATH"
version = 1

[install]
hello.pkg-path = "hello"
EOF

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json" \
    run "$FLOX_BIN" edit -f "$TMP_MANIFEST_PATH"
  assert_success
  assert_output --partial "✅ Environment successfully updated."
}

# ---------------------------------------------------------------------------- #

# bats test_tags=managed,pull,managed:pull
@test "m4: pushed environment can be pulled" {
  mkdir a a_data
  mkdir b b_data

  # on machine a, create and push the environment
  export FLOX_DATA_DIR="$(pwd)/a_data"
  pushd a > /dev/null || return
  "$FLOX_BIN" init
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json" \
    "$FLOX_BIN" install hello
  "$FLOX_BIN" push --owner "$OWNER"
  popd > /dev/null || return

  # on another b machine, pull the environment
  export FLOX_DATA_DIR="$(pwd)/b_data"
  pushd b > /dev/null || return
  "$FLOX_BIN" pull --remote "$OWNER/a"
  run --separate-stderr "$FLOX_BIN" list --name

  # assert that the environment contains the installed package
  assert_output "hello"
  popd > /dev/null || return
}

# bats test_tags=managed,update,managed:update
@test "m5: updated environment can be pulled" {
  mkdir a a_data
  mkdir b b_data

  # on machine a, create and push the (empty) environment
  export FLOX_DATA_DIR="$(pwd)/a_data"
  pushd a > /dev/null || return
  "$FLOX_BIN" init
  "$FLOX_BIN" push --owner "$OWNER"
  popd > /dev/null || return

  # on another b machine,
  #  - pull the environment
  #  - install a package
  #  - push the environment
  export FLOX_DATA_DIR="$(pwd)/b_data"
  pushd b > /dev/null || return
  "$FLOX_BIN" pull --remote "$OWNER/a"
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json" \
    "$FLOX_BIN" install hello
  "$FLOX_BIN" push --owner "$OWNER"
  popd > /dev/null || return

  # on machine a, pull the environment
  # and check that the package is installed
  export FLOX_DATA_DIR="$(pwd)/a_data"
  pushd a > /dev/null || return
  # assert that pulling succeeds
  run "$FLOX_BIN" pull
  assert_success

  # assert that the environment contains the installed package
  run --separate-stderr "$FLOX_BIN" list --name
  assert_output "hello"
  popd > /dev/null || return
}

# bats test_tags=managed,diverged,managed:diverged
@test "m7: remote can not be pulled into diverged environment" {
  mkdir a a_data
  mkdir b b_data

  # on machine a, create and push the (empty) environment
  export FLOX_DATA_DIR="$(pwd)/a_data"
  pushd a > /dev/null || return
  "$FLOX_BIN" init
  "$FLOX_BIN" push --owner "$OWNER"
  popd > /dev/null || return

  # on another b machine,
  #  - pull the environment
  #  - install a package
  #  - push the environment
  export FLOX_DATA_DIR="$(pwd)/b_data"
  pushd b > /dev/null || return
  "$FLOX_BIN" pull --remote "$OWNER/a"
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/vim.json" \
    "$FLOX_BIN" install vim
  "$FLOX_BIN" push --owner "$OWNER"
  popd > /dev/null || return

  # on machine a, pull the environment
  # and check that the package is installed
  export FLOX_DATA_DIR="$(pwd)/a_data"
  pushd a > /dev/null || return
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/emacs.json" \
    run "$FLOX_BIN" install emacs
  # assert that pulling fails
  run "$FLOX_BIN" pull
  assert_failure
  # assert that the environment contains the installed package
  assert_output --partial "diverged"

  # assert that pulling with `--force` succeeds
  run "$FLOX_BIN" pull --force
  assert_success

  popd > /dev/null || return
}

# bats test_tags=managed,diverged,managed:diverged-upstream
@test "m8: remote can be force pulled into diverged environment" {
  mkdir a
  mkdir b

  # on machine a, create and push the (empty) environment
  pushd a > /dev/null || return
  "$FLOX_BIN" init
  FLOX_DATA_DIR="$(pwd)/a_data" "$FLOX_BIN" push --owner "$OWNER"
  popd > /dev/null || return

  pushd b > /dev/null || return
  FLOX_DATA_DIR="$(pwd)/b_data" "$FLOX_BIN" pull --remote "$OWNER/a"
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/vim.json" \
    FLOX_DATA_DIR="$(pwd)/b_data" "$FLOX_BIN" install vim
  popd > /dev/null || return

  pushd a > /dev/null || return
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/emacs.json" \
    FLOX_DATA_DIR="$(pwd)/a_data" "$FLOX_BIN" install emacs
  FLOX_DATA_DIR="$(pwd)/a_data" "$FLOX_BIN" push
  popd > /dev/null || return

  pushd b > /dev/null || return
  FLOX_DATA_DIR="$(pwd)/b_data" "$FLOX_BIN" push --force
  popd > /dev/null || return

  pushd a > /dev/null || return
  FLOX_DATA_DIR="$(pwd)/a_data" run "$FLOX_BIN" pull
  assert_failure
  FLOX_DATA_DIR="$(pwd)/a_data" run "$FLOX_BIN" pull --force
  assert_success
  popd > /dev/null || return
}

# ---------------------------------------------------------------------------- #

# bats test_tags=managed,search,managed:search
@test "m8: search works in managed environment" {
  make_empty_remote_env

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/search/hello.json" \
    run "$FLOX_BIN" search hello
  assert_success
}

# ---------------------------------------------------------------------------- #

# bats test_tags=managed,activate,managed:activate
@test "m9: activate works in managed environment" {
  make_empty_remote_env
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json" \
    "$FLOX_BIN" install hello

  export PROJECT_DIR="$(realpath "$PROJECT_DIR")"
  run "$FLOX_BIN" activate --dir "$PROJECT_DIR" -- command -v hello
  assert_success
  assert_output --regexp "${PROJECT_DIR}/.flox/run/${NIX_SYSTEM}.${PROJECT_NAME}.dev/bin/hello"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=managed,delete,managed:delete
@test "m10: deletes existing environment" {
  # This test asserts before and after state of the home directory.
  # Remaining state from other tests may cause this test misbehave.
  # Hence, use a clean home directory, for this test rather than the shared one.
  home_setup test

  # Note: this creates two envs in one entry in the registry:
  # 1. The initial env created by `flox init`
  # 2. The managed environment created by pushing the path environment
  make_empty_remote_env

  run dot_flox_exists
  assert_success

  # After this we're still left with the path environment
  run "$FLOX_BIN" delete
  assert_success

  run dot_flox_exists
  assert_failure

  # We should only see the path environnment
  run jq '.entries[0].envs | length' "$FLOX_DATA_DIR/env-registry.json"
  assert_output "1"

  rm -rf "$FLOX_CACHE_DIR"
}

# bats test_tags=managed,delete,managed:fresh-deleted
@test "m11: uses fresh branch after delete" {
  make_empty_remote_env
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/vim.json" \
    "$FLOX_BIN" install vim

  run "$FLOX_BIN" delete
  assert_success

  run dot_flox_exists
  assert_failure

  # when recreating an environment, a new branch should be used
  run "$FLOX_BIN" pull --remote "$OWNER/project-managed-${BATS_TEST_NUMBER}"
  assert_success

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/emacs.json" \
    "$FLOX_BIN" install emacs
  run "$FLOX_BIN" list --name
  assert_output --partial "emacs"
  refute_output "vim"
}


# ---------------------------------------------------------------------------- #

# bats test_tags=managed,managed:local-edits-block:install
@test "changes to the local environment block 'flox install'" {
  make_empty_remote_env

  tomlq -i -t '.install.hello."pkg-path" = "hello"' .flox/env/manifest.toml

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/vim.json" \
    run "$FLOX_BIN" install vim
  assert_failure

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json" \
    "$FLOX_BIN" edit --sync

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/vim.json" \
    run "$FLOX_BIN" install vim
  assert_success
}

# bats test_tags=managed,managed:local-edits-block:uninstall
@test "changes to the local environment block 'flox uninstall'"  {
  make_empty_remote_env

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/vim.json" \
    "$FLOX_BIN" install vim

  tomlq -i -t '.install.hello."pkg-path" = "hello"' .flox/env/manifest.toml

  run "$FLOX_BIN" uninstall vim
  assert_failure

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json" \
    "$FLOX_BIN" edit --sync

  run "$FLOX_BIN" uninstall vim
  assert_success
}

# bats test_tags=managed,managed:local-edits-block:upgrade
@test "changes to the local environment block 'flox upgrade'"  {
  make_empty_remote_env

  tomlq -i -t '.install.hello."pkg-path" = "hello"' .flox/env/manifest.toml

  run "$FLOX_BIN" upgrade
  assert_failure

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json" \
    "$FLOX_BIN" edit --sync

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json" \
    run "$FLOX_BIN" upgrade
  assert_success
}

# bats test_tags=managed,managed:local-edits-block:edit
@test "'flox edit' works despite local changes and commits them" {
  make_empty_remote_env

  tomlq -i -t '.install.hello."pkg-path" = "hello"' .flox/env/manifest.toml

  # simulate immediate save in a user editor
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json" \
    run "$FLOX_BIN" edit -f .flox/env/manifest.toml


  assert_success
}

# bats test_tags=managed,managed:local-edits-block:push
@test "changes to the local environment block 'flox push'"  {
  make_empty_remote_env

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/vim.json" \
    "$FLOX_BIN" install vim

  tomlq -i -t '.install.hello."pkg-path" = "hello"' .flox/env/manifest.toml

  run "$FLOX_BIN" push
  assert_failure

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json" \
    "$FLOX_BIN" edit --sync

  run "$FLOX_BIN" push
  assert_success
}

# bats test_tags=managed,managed:local-edits-block:pull
@test "changes to the local environment block 'flox pull'"  {
  make_empty_remote_env

  tomlq -i -t '.install.hello."pkg-path" = "hello"' .flox/env/manifest.toml

  run "$FLOX_BIN" pull
  assert_failure

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json" \
    "$FLOX_BIN" edit --reset

  run "$FLOX_BIN" pull
  assert_success
}

# bats test_tags=managed,managed:local-edits-block:pull-force
@test "changes to the local environment are discarded with 'flox pull --force'" {
  make_empty_remote_env

  tomlq -i -t '.install.hello."pkg-path" = "hello"' .flox/env/manifest.toml

  run "$FLOX_BIN" pull
  assert_failure

  run "$FLOX_BIN" pull --force
  assert_success

  run tomlq '.install.hello' .flox/env/manifest.toml
  assert_output 'null'
}

# bats test_tags=managed,managed:activates-local-edits
@test "'flox activate' activates local edits" {
  make_empty_remote_env

  tomlq -i -t '.install.hello."pkg-path" = "hello"' .flox/env/manifest.toml

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json" \
    run "$FLOX_BIN" activate -- hello

  assert_success

  # after resetting uses the original empty env
  "$FLOX_BIN" edit --reset

  run -127 "$FLOX_BIN" activate -- hello
  assert_failure
}

# bats test_tags=managed,managed:edit-reset
@test "'flox edit --reset' resets local edits" {
  make_empty_remote_env

  tomlq -i -t '.install.hello."pkg-path" = "hello"' .flox/env/manifest.toml

  # after resetting uses the original empty env
  "$FLOX_BIN" edit --reset

  run tomlq '.install.hello' .flox/env/manifest.toml
  assert_output 'null'
}

# bats test_tags=activate,activate:attach
@test "managed environments can attach" {
  project_setup
  export OWNER="owner"
  floxhub_setup "$OWNER"

  "$FLOX_BIN" init
  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1
    [hook]
    on-activate = """
      echo "sourcing hook.on-activate"
    """
EOF
  )"
  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f -
  "$FLOX_BIN" push --owner "$OWNER"

  mkfifo started
  # Will get cat'ed in teardown
  TEARDOWN_FIFO="$PROJECT_DIR/finished"
  mkfifo "$TEARDOWN_FIFO"

  "$FLOX_BIN" activate -- bash -c "echo > started && echo > \"$TEARDOWN_FIFO\"" >> output 2>&1 &
  timeout 2 cat started
  run cat output
  assert_success
  assert_output --partial "sourcing hook.on-activate"


  run "$FLOX_BIN" activate -- true
  assert_success
  refute_output --partial "sourcing hook.on-activate"
}
