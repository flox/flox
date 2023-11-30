#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# `pkgdb manifest' tests related to package groups.
#
# ---------------------------------------------------------------------------- #

load setup_suite.bash;

# bats file_tags=resolver:manifest, resolver:groups

setup_file() {
  export PROJ2="$TESTS_DIR/harnesses/proj2";

  # We don't parallelize these to avoid DB sync headaches and to recycle the
  # cache between tests.
  # Nonetheless this file makes an effort to avoid depending on past state in
  # such a way that would make it difficult to eventually parallelize in
  # the future.
  export BATS_NO_PARALLELIZE_WITHIN_FILE=true;

  # Extract revisions from the manifest.
  STABLE_REV="$(
    yj -t <"$PROJ2/manifest.toml"|jq -r '.registry.inputs.stable.from.rev';
  )";
  STAGING_REV="$(
    yj -t <"$PROJ2/manifest.toml"|jq -r '.registry.inputs.staging.from.rev';
  )";
  UNSTABLE_REV="$(
    yj -t <"$PROJ2/manifest.toml"|jq -r '.registry.inputs.unstable.from.rev';
  )";
  export STABLE_REV STAGING_REV UNSTABLE_REV;
}


# ---------------------------------------------------------------------------- #

# Create a directory with a JSON manifest file based on `proj2/manifest.toml'.
setup_project() {
  local _dir;
  _dir="${1:-$BATS_TEST_TMPDIR/project}";
  mkdir -p "$_dir";
  pushd "$_dir" >/dev/null||return;
  yj -t <"$PROJ2/manifest.toml" > manifest.json;
}


# ---------------------------------------------------------------------------- #

# Edit a JSON file with `jq' in-place.
jq_edit() {
  local _file="${1?You must provide a target file}";
  local _command="${2?You must provide a jq command}";
  local _tmp;
  _tmp="${_file}~";
  jq "$_command" "$_file" >"$_tmp";
  mv "$_tmp" "$_file";
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
  setup_project;

  run sh -c 'pkgdb manifest lock manifest.json > manifest.lock;';
  assert_success;

  run jq -r '.packages["x86_64-linux"].nodejsOld.input.attrs.rev' manifest.lock;
  assert_success;
  assert_output "$STABLE_REV";

  run jq -r '.packages["x86_64-linux"].nodejs.input.attrs.rev' manifest.lock;
  assert_success;
  assert_output "$STAGING_REV";

  run jq -r '.packages["x86_64-linux"].nodejsNew.input.attrs.rev' manifest.lock;
  assert_success;
  assert_output "$UNSTABLE_REV";
}


# ---------------------------------------------------------------------------- #

# bats test_tags=resolver:no-lockfile, resolver:groups

# It is not possible for all three to resolve to a single revision,
# so we expect failure here.
@test "'pkgdb manifest lock' impossible group" {
  setup_project;

  jq_edit manifest.json '.install.nodejsOld|=del( .["package-group"] )
                         |.install.nodejsNew|=del( .["package-group"] )';

  run sh -c 'pkgdb manifest lock manifest.json > manifest.lock;';
  assert_failure;
}


# ---------------------------------------------------------------------------- #

# bats test_tags=resolver:no-lockfile, resolver:groups

# We can get two of our descriptors in a single revision, but not all three.
@test "'pkgdb manifest lock' groups with no previous lock" {
  setup_project;

  jq_edit manifest.json '.install.nodejsNew|=del( .["package-group"] )';

  run sh -c 'pkgdb manifest lock manifest.json > manifest.lock;';
  assert_success;

  run jq -r '.packages["x86_64-linux"].nodejsOld.input.attrs.rev' manifest.lock;
  assert_success;
  assert_output "$STABLE_REV";

  run jq -r '.packages["x86_64-linux"].nodejs.input.attrs.rev' manifest.lock;
  assert_success;
  assert_output "$UNSTABLE_REV";

  run jq -r '.packages["x86_64-linux"].nodejsNew.input.attrs.rev' manifest.lock;
  assert_success;
  assert_output "$UNSTABLE_REV";
}


# ---------------------------------------------------------------------------- #

# bats test_tags=resolver:lockfile, resolver:groups, resolver:optional

# XXX: This test case shows an undesirable behavior.
# Use it as a case study for making improvements and later, remove it or modify
# it to reflect the new behavior.

# Like the test above but adds `nodejsNew' after the lock is created.
# This changes the resolution of `nodejs' to use _staging_ instead of
# _unstable_, making it impossible to resolve `nodejsNew' later.
@test "'pkgdb manifest lock' impossible group with previous lock" {
  setup_project;

  jq_edit manifest.json '.install|=del( .nodejsNew )';

  run sh -c 'pkgdb manifest lock manifest.json|tee manifest.lock;';
  assert_success;

  run jq -r '.packages["x86_64-linux"].nodejsOld.input.attrs.rev' manifest.lock;
  assert_success;
  assert_output "$STABLE_REV";

  run jq -r '.packages["x86_64-linux"].nodejs.input.attrs.rev' manifest.lock;
  assert_success;
  assert_output "$STAGING_REV";

  jq_edit manifest.json '.install.nodejsNew={
    "name": "nodejs", "version": "^18.17"
  }';

  # This doesn't have `pipefail' so we will always get a `manifest.lock2'
  # even if resolution fails.
  run sh -c 'pkgdb manifest lock --lockfile manifest.lock manifest.json  \
               |tee manifest.lock2;';
  assert_success;

  run jq -r '.category_message' manifest.lock2;
  assert_output "resolution failure";

  # Making the package optional fixes makes it possible to resolve.
  jq_edit manifest.json '.install.nodejsNew.optional=true';
  run sh -c 'pkgdb manifest lock --lockfile manifest.lock manifest.json  \
               |tee manifest.lock3;';
  assert_success;

  run jq -r '.packages["x86_64-linux"].nodejsNew' manifest.lock3;
  assert_success;
  assert_output 'null';
}


# ---------------------------------------------------------------------------- #

# bats test_tags=resolver:lockfile, resolver:groups

# Like the test above but adds `nodejs' after the lock is created.
@test "'pkgdb manifest lock' group with previous lock" {
  setup_project;

  jq_edit manifest.json '.install|=del( .nodejs )
                         |.install.nodejsNew|=del( .["package-group"] )';

  run sh -c 'pkgdb manifest lock manifest.json|tee manifest.lock;';
  assert_success;

  run jq -r '.packages["x86_64-linux"].nodejsOld.input.attrs.rev' manifest.lock;
  assert_success;
  assert_output "$STABLE_REV";

  run jq -r '.packages["x86_64-linux"].nodejsNew.input.attrs.rev' manifest.lock;
  assert_success;
  assert_output "$UNSTABLE_REV";

  jq_edit manifest.json '.install.nodejs={
    "name": "nodejs", "version": ">=18.15.0 <19.0.0"
  }';

  # This doesn't have `pipefail' so we will always get a `manifest.lock2'
  # even if resolution fails.
  run sh -c 'pkgdb manifest lock --lockfile manifest.lock manifest.json  \
               |tee manifest.lock2;';
  assert_success;

  run jq -r '.packages["x86_64-linux"].nodejs.input.attrs.rev' manifest.lock2;
  assert_success;
  assert_output "$UNSTABLE_REV";
}


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
