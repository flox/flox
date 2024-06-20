#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test of rust impl of 'flox show'
#
# bats file_tags=search,show
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# ---------------------------------------------------------------------------- #

# Helpers for project based tests.
# Note in this file, these aren't added to setup() and teardown()

project_setup() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/test"
  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" >/dev/null || return
  run "$FLOX_BIN" init
  assert_success
  unset output
  export LOCKFILE_PATH="$PROJECT_DIR/.flox/env/manifest.lock"
}

project_teardown() {
  popd >/dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
  unset LOCKFILE_PATH
}

# ---------------------------------------------------------------------------- #

setup() {
  common_test_setup
  setup_isolated_flox
  rm -f "$GLOBAL_MANIFEST_LOCK"
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.json"
}

teardown() {
  common_test_teardown
}

setup_file() {
  :
}

# ---------------------------------------------------------------------------- #

@test "'flox show' can be called at all" {
  export FLOX_FEATURES_USE_CATALOG=false
  run "$FLOX_BIN" show hello
  assert_success
}

# ---------------------------------------------------------------------------- #

@test "catalog: 'flox show' can be called at all" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/show/hello.json"
  run "$FLOX_BIN" show hello
  assert_success
}

# ---------------------------------------------------------------------------- #

@test "'flox show' accepts search output without separator" {
  export FLOX_FEATURES_USE_CATALOG=false
  run "$FLOX_BIN" search hello
  assert_success
  first_result="${lines[0]%% *}"
  run "$FLOX_BIN" show "$first_result"
  assert_success
}

# ---------------------------------------------------------------------------- #

@test "catalog: 'flox show' accepts search output without separator" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA//search/hello.json"
  run "$FLOX_BIN" search hello
  assert_success
  first_result="${lines[0]%% *}"
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/show/hello.json"
  run "$FLOX_BIN" show "$first_result"
  assert_success
}

# ---------------------------------------------------------------------------- #

@test "'flox show' - hello" {
  export FLOX_FEATURES_USE_CATALOG=false
  _PKGDB_GA_REGISTRY_REF_OR_REV="$PKGDB_NIXPKGS_REV_OLD" \
    "$FLOX_BIN" update --global

  run "$FLOX_BIN" show hello
  assert_success
  assert_equal "${lines[0]}" "hello - A program that produces a familiar, friendly greeting"
  assert_equal "${lines[1]}" "    hello@2.12.1"
}

# ---------------------------------------------------------------------------- #

@test "catalog: 'flox show' - hello" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/show/hello.json"
  run "$FLOX_BIN" show hello
  assert_success
  assert_equal "${lines[0]}" "hello - A program that produces a familiar, friendly greeting"
  assert_equal "${lines[1]}" "    hello@2.12.1"
  assert_equal "${lines[2]}" "    hello@2.12"
  assert_equal "${lines[3]}" "    hello@2.10 (aarch64-linux, x86_64-darwin, x86_64-linux only)"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=python

@test "'flox show' - python27Full" {
  export FLOX_FEATURES_USE_CATALOG=false
  _PKGDB_GA_REGISTRY_REF_OR_REV="$PKGDB_NIXPKGS_REV_OLD" \
    "$FLOX_BIN" update --global

  run "$FLOX_BIN" show python27Full
  assert_success
  assert_equal "${lines[0]}" "python27Full - A high-level dynamically-typed programming language"
  assert_equal "${lines[1]}" "    python27Full@2.7.18.6"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=python

@test "'flox show' - python310Packages.flask" {
  export FLOX_FEATURES_USE_CATALOG=false
  _PKGDB_GA_REGISTRY_REF_OR_REV="$PKGDB_NIXPKGS_REV_OLD" \
    "$FLOX_BIN" update --global

  run "$FLOX_BIN" show python310Packages.flask
  assert_success
  # Ensure that the package and part of the description show up
  assert_output --partial 'python310Packages.flask - The'
}

# ---------------------------------------------------------------------------- #

# bats test_tags=python

# Check pkg-path is handled correctly
@test "catalog: 'flox show' - python310Packages.flask" {
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/show/flask.json" \
    run "$FLOX_BIN" show python310Packages.flask
  assert_success
  # Ensure that the package and part of the description show up
  assert_output --partial 'python310Packages.flask - The'
}

# ---------------------------------------------------------------------------- #

# bats test_tags=ruby

@test "'flox show' - rubyPackages.rails" {
  export FLOX_FEATURES_USE_CATALOG=false
  _PKGDB_GA_REGISTRY_REF_OR_REV="$PKGDB_NIXPKGS_REV_OLD" \
    "$FLOX_BIN" update --global

  run "$FLOX_BIN" show rubyPackages.rails
  assert_success
  assert_output --partial 'rubyPackages.rails - '
}

# ---------------------------------------------------------------------------- #

@test "'flox show' works in project without manifest or lockfile" {
  export FLOX_FEATURES_USE_CATALOG=false
  _PKGDB_GA_REGISTRY_REF_OR_REV="$PKGDB_NIXPKGS_REV_OLD" \
    "$FLOX_BIN" update --global

  project_setup

  rm -f "$PROJECT_DIR/.flox/manifest.toml"
  run --separate-stderr "$FLOX_BIN" show hello
  assert_success

  project_teardown
}

# ---------------------------------------------------------------------------- #

# bats test_tags=search:project, search:manifest, search:show

@test "'flox show' uses '_PKGDB_GA_REGISTRY_REF_OR_REV' revision" {
  export FLOX_FEATURES_USE_CATALOG=false
  _PKGDB_GA_REGISTRY_REF_OR_REV="$PKGDB_NIXPKGS_REV_OLD" \
    "$FLOX_BIN" update --global

  project_setup

  rm -f "$GLOBAL_MANIFEST_LOCK"

  mkdir -p "$PROJECT_DIR/.flox/env"
  # Note: at some point it may also be necessary to create a .flox/env.json
  echo 'options.systems = ["x86_64-linux"]' \
    >"$PROJECT_DIR/.flox/env/manifest.toml"

  # Search for a package with `pkgdb`
  run --separate-stderr sh -c "$PKGDB_BIN search --ga-registry '{
      \"manifest\": \"$PROJECT_DIR/.flox/env/manifest.toml\",
      \"query\": { \"match-name\": \"nodejs\" }
    }'|head -n1|jq -r '.version';"
  assert_success
  assert_output "$NODEJS_VERSION_NEW"
  unset output

  # Ensure the version of `nodejs' in our search results aligns with that in
  # _PKGDB_GA_REGISTRY_REF_OR_REV.
  run --separate-stderr sh -c "$FLOX_BIN show nodejs|tail -n1"
  assert_success
  assert_output "    nodejs@$NODEJS_VERSION_NEW"

  project_teardown
}

# ---------------------------------------------------------------------------- #

# bats test_tags=search:project, search:manifest, search:lockfile, search:show

@test "'flox show' uses locked revision when available" {
  export FLOX_FEATURES_USE_CATALOG=false
  _PKGDB_GA_REGISTRY_REF_OR_REV="$PKGDB_NIXPKGS_REV_OLD" \
    "$FLOX_BIN" update --global
  project_setup

  mkdir -p "$PROJECT_DIR/.flox/env"
  # Note: at some point it may also be necessary to create a .flox/env.json
  {
    echo 'options.systems = ["x86_64-linux"]'
    echo 'install.nodejs = {}'
  } >"$PROJECT_DIR/.flox/env/manifest.toml"

  # Force lockfile to pin a specific revision of `nixpkgs'
  run --separate-stderr sh -c \
    "_PKGDB_GA_REGISTRY_REF_OR_REV='${PKGDB_NIXPKGS_REV_OLD?}'          \
      $PKGDB_BIN manifest lock                                         \
                 --ga-registry                                         \
                 --manifest '$PROJECT_DIR/.flox/env/manifest.toml'     \
                 > '$PROJECT_DIR/.flox/env/manifest.lock';"
  assert_success
  unset output

  # Ensure the locked revision is what we expect.
  run --separate-stderr jq -r '.registry.inputs.nixpkgs.from.rev' \
    "$PROJECT_DIR/.flox/env/manifest.lock"
  assert_success
  assert_output "$PKGDB_NIXPKGS_REV_OLD"
  unset output

  # Search for a package with `pkgdb`
  run --separate-stderr sh -c \
    "_PKGDB_GA_REGISTRY_REF_OR_REV='$PKGDB_NIXPKGS_REV_OLD'       \
      $PKGDB_BIN search --ga-registry '{
        \"manifest\": \"$PROJECT_DIR/.flox/env/manifest.toml\",
        \"lockfile\": \"$PROJECT_DIR/.flox/env/manifest.lock\",
        \"query\": { \"match-name\": \"nodejs\" }
      }'|head -n1|jq -r '.version';"
  assert_success
  assert_output "$NODEJS_VERSION_OLD"
  unset output

  # Ensure the version of `nodejs' in our search results aligns with the
  # locked rev, instead of the `--ga-registry` default.
  run --separate-stderr sh -c "$FLOX_BIN show nodejs|tail -n1"
  assert_success
  assert_output "    nodejs@$NODEJS_VERSION_OLD"

  project_teardown
}

# ---------------------------------------------------------------------------- #

@test "'flox show' creates global lock" {
  export FLOX_FEATURES_USE_CATALOG=false
  rm -f "$GLOBAL_MANIFEST_LOCK"
  run ! [ -e "$LOCKFILE_PATH" ]
  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_OLD?}" \
    run --separate-stderr sh -c "$FLOX_BIN show nodejs|tail -n1"
  assert_success
  assert_output "    nodejs@$NODEJS_VERSION_OLD"

  # Check the expected global lock was created
  run jq -r '.registry.inputs.nixpkgs.from.narHash' "$GLOBAL_MANIFEST_LOCK"
  assert_success
  assert_output "$PKGDB_NIXPKGS_NAR_HASH_OLD"
}

# ---------------------------------------------------------------------------- #

@test "'flox show' uses global lock" {
  export FLOX_FEATURES_USE_CATALOG=false
  rm -f "$GLOBAL_MANIFEST_LOCK"
  run ! [ -e "$LOCKFILE_PATH" ]
  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_OLD?}" \
    "$FLOX_BIN" update --global

  # Set new rev just to make sure we're not incidentally using old rev.
  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_NEW?}" \
    run --separate-stderr sh -c "$FLOX_BIN show nodejs|tail -n1"
  assert_success
  assert_output '    nodejs@18.16.0'

}

# ---------------------------------------------------------------------------- #
