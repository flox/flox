#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test of rust impl of 'flox list'
#
# Tests are tentative, missing spec!
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# ---------------------------------------------------------------------------- #

# Helpers for project based tests.

project_setup() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/test"
  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" >/dev/null || return

  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.json"
}

project_teardown() {
  popd >/dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
}

# ---------------------------------------------------------------------------- #

setup() {
  common_test_setup
  project_setup
}
teardown() {
  project_teardown
  common_test_teardown
}

# ---------------------------------------------------------------------------- #

init_pkgdb_env() {
  mkdir -p "$PROJECT_DIR/.flox/env"
  cp --no-preserve=mode "$MANUALLY_GENERATED"/empty_v0/* "$PROJECT_DIR/.flox/env"

  echo '{
    "name": "env",
    "version": 1
  }' >>"$PROJECT_DIR/.flox/env.json"
}

# ---------------------------------------------------------------------------- #

@test "'flox list' lists packages of environment in the current dir; fails if no env found" {
  run "$FLOX_BIN" list
  assert_failure
}

@test "'flox list' lists packages of environment in the current dir; No package" {
  init_pkgdb_env
  run "$FLOX_BIN" list
  assert_success
}

@test "'flox list' lists packages of environment in the current dir; One package from nixpkgs" {
  init_pkgdb_env
  cp "$MANUALLY_GENERATED"/hello_v0/* "$PROJECT_DIR/.flox/env"

  run "$FLOX_BIN" list
  assert_success
  assert_output --regexp 'hello: hello \([0-9]+\.[0-9]+(\.[0-9]+)?\)'
}

@test "'flox list' lists packages of environment in the current dir; shows different paths" {
  init_pkgdb_env
  cp "$MANUALLY_GENERATED"/python_v0/* "$PROJECT_DIR/.flox/env"

  run "$FLOX_BIN" list
  assert_success
  assert_output --regexp - <<EOF
pip: python311Packages.pip \([0-9]+\.[0-9]+(\.[0-9]+)?\)
EOF
}

@test "'flox list' lists packages of environment in the current dir; shows different id" {
  init_pkgdb_env

  # install hello with `greeting` as the iid.
  cp "$MANUALLY_GENERATED"/hello_as_greeting_v0/* "$PROJECT_DIR/.flox/env"

  run "$FLOX_BIN" list
  assert_success
  assert_output --regexp - <<EOF
greeting: hello \([0-9]+\.[0-9]+(\.[0-9]+)?\)
EOF
}

# bats test_tags=list,list:config
@test "'flox list --config' shows manifest content" {
  init_pkgdb_env
  # mock files are copied from store with ro permissions
  tomlq -i -t '.hook."on-activate" = "something suspicious"' "$PROJECT_DIR/.flox/env/manifest.toml"

  MANIFEST_CONTENT="$(
    cat <<EOF
[hook]
on-activate = "something suspicious"
EOF
  )"

  run "$FLOX_BIN" list --config
  assert_success
  assert_output --partial "$MANIFEST_CONTENT"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=list,list:not-applicable
@test "'flox list' hides packages not installed for the current system" {
  init_pkgdb_env

  # Mock env with `hello` installed for all systems
  # and `htop` for no system to emulate a package not installed
  # for the current system on all systems.
  cp "$MANUALLY_GENERATED"/hello_and_htop_for_no_system_v0/* "$PROJECT_DIR/.flox/env"

  run "$FLOX_BIN" list -n
  assert_success
  assert_output "hello"
}

# ---------------------------------------------------------------------------- #

# https://github.com/flox/flox/issues/1039
# bats test_tags=list,list:tolerates-missing-version
@test "'flox list' tolerates missing version" {
  init_pkgdb_env

  # `influxdb2 does not have a version attribute set in nixpkgs (2024-02-19)
  # todo: replace with a more predicatable/smaller example
  cp "$MANUALLY_GENERATED"/influxdb2_v0/* "$PROJECT_DIR/.flox/env"

  run "$FLOX_BIN" list
  assert_success
  assert_output "influxdb2: influxdb2 (N/A)"
}

# ------------------------------ Catalog Tests ------------------------------- #
# ---------------------------------------------------------------------------- #

# bats test_tags=list,list:catalog
@test "catalog: 'flox list' lists packages of environment in the current dir; One package from nixpkgs" {
  "$FLOX_BIN" init
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json" \
    "$FLOX_BIN" install hello

  run "$FLOX_BIN" list
  assert_success
  assert_output --regexp 'hello: hello \([0-9]+\.[0-9]+(\.[0-9]+)?\)'
}

# bats test_tags=list,list:catalog,list:config
@test "catalog: 'flox list --config' shows manifest content" {
  "$FLOX_BIN" init
  MANIFEST_CONTENTS="$(
    cat <<-EOF
    version = 1

    [install]

    [hook]
    on-activate = "something suspicious"
EOF
  )"

  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f -

  run "$FLOX_BIN" list --config
  assert_success
  assert_output "$MANIFEST_CONTENTS"
}

# ---------------------------------------------------------------------------- #
