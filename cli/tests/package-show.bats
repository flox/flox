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
  pushd "$PROJECT_DIR" > /dev/null || return
  run "$FLOX_BIN" init
  assert_success
  unset output
  export LOCKFILE_PATH="$PROJECT_DIR/.flox/env/manifest.lock"
}

project_teardown() {
  popd > /dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
  unset LOCKFILE_PATH
}

# ---------------------------------------------------------------------------- #

setup() {
  common_test_setup
}
teardown() {
  common_test_teardown
}

setup_file() {
  rm -f "$GLOBAL_MANIFEST_LOCK"
  _PKGDB_GA_REGISTRY_REF_OR_REV="$PKGDB_NIXPKGS_REV_OLD" \
    "$FLOX_BIN" update --global
}

# ---------------------------------------------------------------------------- #

@test "'flox show' can be called at all" {
  run "$FLOX_BIN" show hello
  assert_success
}

# ---------------------------------------------------------------------------- #

@test "'flox show' accepts specific input" {
  skip DEPRECATED
  run "$FLOX_BIN" show nixpkgs-flox:hello
  assert_success
  # TODO: better testing once the formatting is implemented
}

# ---------------------------------------------------------------------------- #

@test "'flox show' accepts search output without separator" {
  run "$FLOX_BIN" search hello
  assert_success
  first_result="${lines[0]%% *}"
  run "$FLOX_BIN" show "$first_result"
  assert_success
}

# ---------------------------------------------------------------------------- #

@test "'flox show' accepts search output with separator" {
  skip DEPRECATED
  run "$FLOX_BIN" search nixpkgs-flox:hello
  assert_success
  first_result="${lines[0]%% *}"
  run "$FLOX_BIN" show "$first_result"
  assert_success
}

# ---------------------------------------------------------------------------- #

@test "'flox show' - hello" {
  run "$FLOX_BIN" show hello
  assert_success
  assert_equal "${lines[0]}" "hello - A program that produces a familiar, friendly greeting"
  assert_equal "${lines[1]}" "    hello - hello@2.12.1"
}

# ---------------------------------------------------------------------------- #

@test "'flox show' - hello --all" {
  run "$FLOX_BIN" show hello --all
  assert_success
  assert_equal "${lines[0]}" "hello - A program that produces a familiar, friendly greeting"
  assert_equal "${lines[1]}" "    hello - hello@2.12.1"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=python

@test "'flox show' - python27Full" {
  run "$FLOX_BIN" show python27Full
  assert_success
  assert_equal "${lines[0]}" "python27Full - A high-level dynamically-typed programming language"
  assert_equal "${lines[1]}" "    python27Full - python27Full@2.7.18.6"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=python

@test "'flox show' - python27Full --all" {
  run "$FLOX_BIN" show python27Full --all
  assert_success
  assert_equal "${lines[0]}" "python27Full - A high-level dynamically-typed programming language"
  assert_equal "${lines[1]}" "    python27Full - python27Full@2.7.18.6"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=python

@test "'flox show' - python310Packages.flask" {
  run "$FLOX_BIN" show python310Packages.flask
  assert_success
  # Ensure that the package and part of the description show up
  assert_output --partial 'python310Packages.flask - The'
}

# ---------------------------------------------------------------------------- #

# bats test_tags=ruby

@test "'flox show' - rubyPackages.rails" {
  run "$FLOX_BIN" show rubyPackages.rails
  assert_success
  assert_output --partial 'rubyPackages.rails - '
}

# ---------------------------------------------------------------------------- #

@test "'flox show' works in project without manifest or lockfile" {
  project_setup

  rm -f "$PROJECT_DIR/.flox/manifest.toml"
  run --separate-stderr "$FLOX_BIN" show hello
  assert_success

  project_teardown
}

# ---------------------------------------------------------------------------- #

# bats test_tags=search:project, search:manifest, search:show

@test "'flox show' uses '_PKGDB_GA_REGISTRY_REF_OR_REV' revision" {
  project_setup

  rm -f "$GLOBAL_MANIFEST_LOCK"

  mkdir -p "$PROJECT_DIR/.flox/env"
  # Note: at some point it may also be necessary to create a .flox/env.json
  echo 'options.systems = ["x86_64-linux"]' \
    > "$PROJECT_DIR/.flox/env/manifest.toml"

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
  assert_output "    nodejs - nodejs@$NODEJS_VERSION_NEW"

  project_teardown
}

# ---------------------------------------------------------------------------- #

# bats test_tags=search:project, search:manifest, search:lockfile, search:show

@test "'flox show' uses locked revision when available" {
  project_setup

  mkdir -p "$PROJECT_DIR/.flox/env"
  # Note: at some point it may also be necessary to create a .flox/env.json
  {
    echo 'options.systems = ["x86_64-linux"]'
    echo 'install.nodejs = {}'
  } > "$PROJECT_DIR/.flox/env/manifest.toml"

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
  assert_output "    nodejs - nodejs@$NODEJS_VERSION_OLD"

  project_teardown
}

# ---------------------------------------------------------------------------- #

@test "'flox show' creates global lock" {
  rm -f "$GLOBAL_MANIFEST_LOCK"
  run ! [ -e "$LOCKFILE_PATH" ]
  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_OLD?}" \
    run --separate-stderr sh -c "$FLOX_BIN show nodejs|tail -n1"
  assert_success
  assert_output "    nodejs - nodejs@$NODEJS_VERSION_OLD"

  # Check the expected global lock was created
  run jq -r '.registry.inputs.nixpkgs.from.narHash' "$GLOBAL_MANIFEST_LOCK"
  assert_success
  assert_output "$PKGDB_NIXPKGS_NAR_HASH_OLD"
}

# ---------------------------------------------------------------------------- #

@test "'flox show' uses global lock" {
  rm -f "$GLOBAL_MANIFEST_LOCK"
  run ! [ -e "$LOCKFILE_PATH" ]
  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_OLD?}" \
    "$FLOX_BIN" update --global

  # Set new rev just to make sure we're not incidentally using old rev.
  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_NEW?}" \
    run --separate-stderr sh -c "$FLOX_BIN show nodejs|tail -n1"
  assert_success
  assert_output '    nodejs - nodejs@18.16.0'

}

# ---------------------------------------------------------------------------- #

@test "'flox show' prompts when an environment is activated and there is an environment in the current directory" {
  # Set up two environments locked to different revisions of nixpkgs, and
  # confirm that flox show displays different versions of nodejs for each.

  rm -f "$GLOBAL_MANIFEST_LOCK"

  mkdir 1
  pushd 1
  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_OLD?}" \
    "$FLOX_BIN" init
  "$FLOX_BIN" --debug install nodejs

  run --separate-stderr sh -c "$FLOX_BIN show nodejs|tail -n1"
  assert_success
  assert_output "    nodejs - nodejs@$NODEJS_VERSION_OLD"
  popd

  mkdir 2
  pushd 2
  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_NEW?}" \
    "$FLOX_BIN" update --global

  # new environment uses the global lock
  "$FLOX_BIN" init
  "$FLOX_BIN" install nodejs

  run --separate-stderr sh -c "$FLOX_BIN show nodejs|tail -n1"
  assert_success
  assert_output "    nodejs - nodejs@$NODEJS_VERSION_NEW"
  popd

  SHELL=bash run expect -d "$TESTS_DIR/show/prompt-which-environment.exp"
  assert_success
  assert_output --partial "nodejs - nodejs@$NODEJS_VERSION_NEW"
}
