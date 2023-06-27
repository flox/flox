#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test the `flox develop' command.
#
# These tests are run in trivial project harnesses.
# It's important for each test case to run in a completely fresh instance of
# that harness because `flox develop' produces files at runtime which may
# pollute later runs.
# The helpers `setup', `teardown', and `loadHarness' streamline this.
#
# ---------------------------------------------------------------------------- #
#
# NOTE: the develop flake may have an out of date lock.
#
# TODO: make parallelizable by using unique directory names.
#
# ---------------------------------------------------------------------------- #

load test_support.bash;

# bats file_tags=develop, project-env, expect


# ---------------------------------------------------------------------------- #

setup_file() {
  common_setup;
  # We can't really parallelize these because we reuse the same test dirs.
  # e.g. `FLOX_TEST_HOME/develop' is used multiple times.
  export BATS_NO_PARALLELIZE_WITHIN_FILE=true;
}


setup() {
  unset HARNESS;
  cd "${FLOX_TEST_HOME?}"||return;
}


teardown() {
  cd "${FLOX_TEST_HOME?}"||return;
  if [[ -n "${HARNESS:-}" ]] && [[ -d "${FLOX_TEST_HOME?}/$HARNESS" ]]; then
    rm -rf "${FLOX_TEST_HOME:?}/$HARNESS";
  fi
}


# ---------------------------------------------------------------------------- #

# Unpack a test harness environment from `<flox>/tests/develop/$1'
# to `$FLOX_TEST_HOME/$1', and changes the current working directory there.

# We use `tar' instead of `cp' to instantiate that sandbox because Darwin
# systems are shipped with the FreeBSD implementation of system utilities -
# unlike the vastly superior GNU `coreutils' implementations, their `cp' lacks
# the ability to dereference symlinks and stuff.
loadHarness() {
  rm -rf "${FLOX_TEST_HOME:?}/$1";
  # Note the use of --dereference to copy flake.{nix,lock} as files.
  tar -cf - --dereference --mode u+w -C "$TESTS_DIR/develop" "./$1"  \
    |tar -C "$FLOX_TEST_HOME" -xf -;
  cd "$FLOX_TEST_HOME/$1"||return;
  export HARNESS="$1";
  # Pre-evaluate targets to avoid non-determinism in `expect' timeouts later.
  $FLOX_CLI nix flake show >/dev/null 2>&1;
}


# A helper that asserts that certain generated files exist.
assertPkgFiles() {
  local _harness _target;
  _harness="${HARNESS:-develop}";
  _target="${1:-default}";
  assert test -h "$FLOX_TEST_HOME/$_harness/.flox/envs/$NIX_SYSTEM.$_target";
  if [[ "$_target" = default ]]; then
    assert test -f "$FLOX_TEST_HOME/$_harness/catalog.json";
    assert test -f "$FLOX_TEST_HOME/$_harness/manifest.json";
  else
    assert test -f "$FLOX_TEST_HOME/$_harness/pkgs/$_target/catalog.json";
    assert test -f "$FLOX_TEST_HOME/$_harness/pkgs/$_target/manifest.json";
  fi
}


runExpect() {
  run expect "$TESTS_DIR/develop/develop.exp" "$@";
  assert_success;
}


# ---------------------------------------------------------------------------- #

@test "'flox develop' from flake root with no installable" {
  loadHarness develop;
  runExpect '';
  assertPkgFiles my-pkg;
}


@test "'flox develop' from flake root with '.#my-pkg'" {
  loadHarness develop;
  runExpect '.#my-pkg';
  assertPkgFiles my-pkg;
}


@test "'flox develop' from flake root with '.#packages.$NIX_SYSTEM.my-pkg'" {
  loadHarness develop;
  runExpect ".#packages.$NIX_SYSTEM.my-pkg";
  assertPkgFiles my-pkg;
}


@test "'flox develop' from flake root with '$FLOX_TEST_HOME/develop#my-pkg'" {
  loadHarness develop;
  runExpect "$FLOX_TEST_HOME/develop#my-pkg";
  assertPkgFiles my-pkg;
}


# ---------------------------------------------------------------------------- #

@test "'flox develop' from flake subdirectory with relative URI" {
  loadHarness develop;
  run cd "$FLOX_TEST_HOME/develop/pkgs";
  assert_success;
  runExpect '.#my-pkg';
  assertPkgFiles my-pkg;
}


@test "'flox develop' from flake subdirectory with absolute URI" {
  loadHarness develop;
  run cd "$FLOX_TEST_HOME/develop/pkgs";
  assert_success;
  runExpect "$FLOX_TEST_HOME/develop#my-pkg";
  assertPkgFiles my-pkg;
}


# ---------------------------------------------------------------------------- #

@test "'flox develop' from parent directory" {
  loadHarness develop;
  run cd "$FLOX_TEST_HOME";
  assert_success;
  runExpect "$FLOX_TEST_HOME/develop#my-pkg";
}


# ---------------------------------------------------------------------------- #

# bats test_tags=git:local
@test "'flox develop' after 'git init' with relative URI" {
  loadHarness develop;
  run git init;
  assert_success;
  run git add .;
  assert_success;
  runExpect ".#my-pkg";
  assertPkgFiles my-pkg;
}

# bats test_tags=git:local
@test "'flox develop' after 'git init' with absolute URI" {
  loadHarness develop;
  run git init;
  assert_success;
  run git add .;
  assert_success;
  runExpect "$FLOX_TEST_HOME/develop#my-pkg";
  assertPkgFiles my-pkg;
}


# ---------------------------------------------------------------------------- #

# bats test_tags=git:remote
@test "'flox develop' fails with remote flake" {
  run expect "$TESTS_DIR/develop/develop-fail.exp"                           \
             "git+ssh://git@github.com/flox/flox?dir=tests/develop#my-pkg";
  assert_success;
}


# ---------------------------------------------------------------------------- #

@test "'flox develop' toplevel with package's default target" {
  loadHarness toplevel-flox-nix-with-pkg;
  runExpect '';
  assertPkgFiles default;
}


# ---------------------------------------------------------------------------- #

@test "'flox develop' toplevel with 'flox install' env" {
  loadHarness toplevel-flox-nix;
  run "$FLOX_CLI" install -e '.#default' hello;
  assert_success;
  # for some reason expect hangs forever when SHELL=zsh and I don't feel like
  # debugging why
  SHELL=bash run expect "$TESTS_DIR/develop/toplevel-flox-nix.exp" '';
  assert_success;
  assertPkgFiles default;
}


# ---------------------------------------------------------------------------- #

# bats test_tags devShell
@test "'flox develop' with 'devShell'" {
  loadHarness devShell;
  run expect "$TESTS_DIR/develop/devShell.exp" '';
  assert_success;
  refute test -h "$FLOX_TEST_HOME/$HARNESS/.flox/envs/$NIX_SYSTEM.default";
  refute test -f "$FLOX_TEST_HOME/$HARNESS/catalog.json";
  refute test -f "$FLOX_TEST_HOME/$HARNESS/manifest.json";
}


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
