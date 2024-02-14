#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# `pkgdb search' tests.
#
# ---------------------------------------------------------------------------- #

load setup_suite.bash

# bats file_tags=search

setup_file() {
  export TDATA="$TESTS_DIR/data/manifest"
  export PROJ1="$TESTS_DIR/harnesses/proj1"

  # We don't parallelize these to avoid DB sync headaches and to recycle the
  # cache between tests.
  # Nonetheless this file makes an effort to avoid depending on past state in
  # such a way that would make it difficult to eventually parallelize in
  # the future.
  export BATS_NO_PARALLELIZE_WITHIN_FILE=true

  # Change the rev used for the `--ga-registry' flag to align with our cached
  # revision used by other tests.
  # This is both an optimization and a way to ensure consistency of test output.
  export _PKGDB_GA_REGISTRY_REF_OR_REV="$NIXPKGS_REV"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=manifest:empty, lock:empty

@test "lock a manifest with system specific packages" {
  _MANIFEST="$BATS_TEST_TMPDIR/manifest.toml";
  echo '[options]
systems = ["x86_64-linux", "x86_64-darwin"]

[install.a]
pkg-path = ["hello"]
systems = ["x86_64-linux"]

[install.b]
pkg-path = ["hello"]
systems = ["x86_64-darwin"]' > "$_MANIFEST";

  run sh -c "$PKGDB_BIN manifest lock --ga-registry --manifest '$_MANIFEST'  \
               > '$BATS_TEST_TMPDIR/manifest.lock'";
  assert_success;

  run sh -c "$PKGDB_BIN manifest lock --ga-registry --manifest '$_MANIFEST'    \
                                 --lockfile '$BATS_TEST_TMPDIR/manifest.lock'  \
               > '$BATS_TEST_TMPDIR/manifest.lock2'";
  assert_success;

  run diff -q "$BATS_TEST_TMPDIR/manifest.lock"    \
              "$BATS_TEST_TMPDIR/manifest.lock2";
  assert_success;
}


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
