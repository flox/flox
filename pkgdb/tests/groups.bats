#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# `pkgdb manifest' tests related to package groups.
#
# ---------------------------------------------------------------------------- #

load setup_suite.bash

# bats file_tags=resolver:manifest, resolver:groups

setup_file() {
  export PROJ2="$TESTS_DIR/harnesses/proj2"

  # We don't parallelize these to avoid DB sync headaches and to recycle the
  # cache between tests.
  # Nonetheless this file makes an effort to avoid depending on past state in
  # such a way that would make it difficult to eventually parallelize in
  # the future.
  export BATS_NO_PARALLELIZE_WITHIN_FILE=true

  STABLE_REV="$NIXPKGS_REV_OLDER"
  STAGING_REV="$NIXPKGS_REV_OLD"
  UNSTABLE_REV="$NIXPKGS_REV"

  export STABLE_REV STAGING_REV UNSTABLE_REV
}

# ---------------------------------------------------------------------------- #

# Create a directory with a JSON manifest file based on `proj2/manifest.toml'.
setup_project() {
  local _dir
  _dir="${1:-$BATS_TEST_TMPDIR/project}"
  mkdir -p "$_dir"
  pushd "$_dir" > /dev/null || return
  yj -t < "$PROJ2/manifest.toml" > manifest.json
}

# ---------------------------------------------------------------------------- #

# Edit a JSON file with `jq' in-place.
jq_edit() {
  local _file="${1?You must provide a target file}"
  local _command="${2?You must provide a jq command}"
  local _tmp
  _tmp="${_file}~"
  jq "$_command" "$_file" > "$_tmp"
  mv "$_tmp" "$_file"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=resolver:no-lockfile, resolver:singleton-groups

# This has three packages in singleton groups.
# One of them is in the default group.
# The other two are in non-default groups.
# It is not possible for all three to resolve to a single revision.
#
# We expect each descriptor to resolve to a different revision when processed
# in separate groups.
@test "'pkgdb manifest lock' singleton groups with no previous lock" {
  setup_project

  run sh -c '$PKGDB_BIN manifest lock --manifest manifest.json > manifest.lock;'
  assert_success

  run jq -r '.packages["x86_64-linux"].nodejsOld.input.attrs.rev' manifest.lock
  assert_success
  assert_output "$STABLE_REV"

  run jq -r '.packages["x86_64-linux"].nodejs.input.attrs.rev' manifest.lock
  assert_success
  assert_output "$STAGING_REV"

  run jq -r '.packages["x86_64-linux"].nodejsNew.input.attrs.rev' manifest.lock
  assert_success
  assert_output "$UNSTABLE_REV"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=resolver:no-lockfile, resolver:groups

# It is not possible for all three to resolve to a single revision,
# so we expect failure here.
@test "'pkgdb manifest lock' impossible group" {
  setup_project

  jq_edit manifest.json '.install.nodejsOld|=del( .["pkg-group"] )
                         |.install.nodejsNew|=del( .["pkg-group"] )'

  run sh -c '$PKGDB_BIN manifest lock --manifest manifest.json > manifest.lock;'
  assert_failure
}

# ---------------------------------------------------------------------------- #

# bats test_tags=resolver:no-lockfile, resolver:groups

# We can get two of our descriptors in a single revision, but not all three.
@test "'pkgdb manifest lock' groups with no previous lock" {
  setup_project

  jq_edit manifest.json '.install.nodejsNew|=del( .["pkg-group"] )'

  run sh -c '$PKGDB_BIN manifest lock --manifest manifest.json > manifest.lock;'
  assert_success

  run jq -r '.packages["x86_64-linux"].nodejsOld.input.attrs.rev' manifest.lock
  assert_success
  assert_output "$STABLE_REV"

  run jq -r '.packages["x86_64-linux"].nodejs.input.attrs.rev' manifest.lock
  assert_success
  assert_output "$UNSTABLE_REV"

  run jq -r '.packages["x86_64-linux"].nodejsNew.input.attrs.rev' manifest.lock
  assert_success
  assert_output "$UNSTABLE_REV"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=resolver:lockfile, resolver:groups, resolver:optional, resolver:upgrade

# XXX: This test case in particular is worth reading closely because it shows
#      a handful of important edge case handling behaviors that are worth
#      reviewing closely.

# Like the test above but adds `nodejsNew' after the lock is created.
# This changes the resolution of `nodejs' to use _staging_ instead of
# _unstable_, making it impossible to resolve `nodejsNew' later with the
# same rev.
# We expect this to succeed by upgrading the entire group and emitting a warning
# for the user.
@test "'pkgdb manifest lock' upgraded group with previous lock" {
  setup_project

  jq_edit manifest.json '.install|=del( .nodejsNew )'

  run sh -c '$PKGDB_BIN manifest lock --manifest manifest.json|tee manifest.lock;'
  assert_success

  run jq -r '.packages["x86_64-linux"].nodejsOld.input.attrs.rev' manifest.lock
  assert_success
  assert_output "$STABLE_REV"

  run jq -r '.packages["x86_64-linux"].nodejs.input.attrs.rev' manifest.lock
  assert_success
  assert_output "$STAGING_REV"

  jq_edit manifest.json '.install.nodejsNew={
    "name": "nodejs", "version": "^'"$NODEJS_VERSION"'"
  }'

  # Making the package optional fixes makes it possible to resolve without
  # an upgrade.
  jq_edit manifest.json '.install.nodejsNew.optional=true'
  run sh -c '$PKGDB_BIN manifest lock --lockfile manifest.lock --manifest manifest.json  \
               |tee manifest.lock2;'
  assert_success
  # Because the package was marked optional, we DO NOT perform an upgrade here!
  run jq -r '.packages["x86_64-linux"].nodejsNew' manifest.lock2
  assert_success
  assert_output 'null'

  # This doesn't have `pipefail' so we will always get a `manifest.lock2'
  # even if resolution fails.
  jq_edit manifest.json '.install.nodejsNew|=del( .optional )'
  run sh -c '$PKGDB_BIN manifest lock --lockfile manifest.lock --manifest manifest.json  \
               |tee manifest.lock3;'
  assert_success
  assert_output --partial "upgrading group 'default'"
  # Ensure we didn't produce an error.
  run jq -r '.category_message' manifest.lock3
  assert_output "null"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=resolver:lockfile, resolver:groups

# Like the test above but adds `nodejs' after the lock is created.
@test "'pkgdb manifest lock' group with previous lock" {
  setup_project

  jq_edit manifest.json '.install|=del( .nodejs )
                         |.install.nodejsNew|=del( .["pkg-group"] )'

  run sh -c '$PKGDB_BIN manifest lock --manifest manifest.json|tee manifest.lock;'
  assert_success

  run jq -r '.packages["x86_64-linux"].nodejsOld.input.attrs.rev' manifest.lock
  assert_success
  assert_output "$STABLE_REV"

  run jq -r '.packages["x86_64-linux"].nodejsNew.input.attrs.rev' manifest.lock
  assert_success
  assert_output "$UNSTABLE_REV"

  jq_edit manifest.json '.install.nodejs={
    "name": "nodejs", "version": ">'"$NODEJS_VERSION_OLDEST"' <='"$NODEJS_VERSION"'"
  }'

  # This doesn't have `pipefail' so we will always get a `manifest.lock2'
  # even if resolution fails.
  run sh -c '$PKGDB_BIN manifest lock --lockfile manifest.lock --manifest manifest.json  \
               |tee manifest.lock2;'
  assert_success

  run jq -r '.packages["x86_64-linux"].nodejs.input.attrs.rev' manifest.lock2
  assert_success
  assert_output "$UNSTABLE_REV"
}

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
