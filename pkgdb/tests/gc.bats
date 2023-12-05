#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# `pkgdb get' CLI tests.
#
# These tests target a specific revision of `nixpkgs` and focus on a small
# package set.
#
# This test relies on `pkgdb scrape` working correctly, and tests will be
# skipped if attempts to produce a shared db fail.
#
#
# ---------------------------------------------------------------------------- #

load setup_suite.bash;

# bats file_tags=cli,gc


# ---------------------------------------------------------------------------- #

setup_file() {
  export DBPATH="$BATS_FILE_TMPDIR/db.sqlite";
  mkdir -p "$BATS_FILE_TMPDIR";
  $PKGDB scrape --database "$DBPATH" "$NIXPKGS_REF"           \
                   legacyPackages "$NIX_SYSTEM" 'akkoma-emoji';
}

setup() {
  cp "$DBPATH" "$BATS_TEST_TMPDIR/current.sqlite";
  cp "$DBPATH" "$BATS_TEST_TMPDIR/stale.sqlite";
  touch -ad "- 4 days" "$BATS_TEST_TMPDIR/stale.sqlite";
}

# ---------------------------------------------------------------------------- #


# ---------------------------------------------------------------------------- #

# bats test_tags=gc:filter-all
@test "pkgdb gc --min-age <old> removes no database" {
  run $PKGDB gc -c "$BATS_TEST_TMPDIR" --min-age 30 ;
  assert_success
  assert_output --partial "Found 0 stale databases.";
}

# bats test_tags=gc:remove-stale
@test "pkgdb gc --min-age <recent> removes 1 database" {
  ls -la "$BATS_TEST_TMPDIR"

  run $PKGDB gc -c "$BATS_TEST_TMPDIR" --min-age 3;
  assert_success
  assert_line --index 0 "Found 1 stale databases.";
  assert_line --index 1 "deleting \"$BATS_TEST_TMPDIR/stale.sqlite\"";

  assert [ ! -f "$BATS_TEST_TMPDIR/stale.sqlite" ] # stale db is removed
  assert [ -f "$BATS_TEST_TMPDIR/current.sqlite" ] # current db is not removed
}

# bats test_tags=gc:dry-run-remove-stale
@test "pkgdb gc --dry-run lists 1 database to remove but does not remove it" {
  ls -la "$BATS_TEST_TMPDIR"

  run $PKGDB gc -c "$BATS_TEST_TMPDIR" --min-age 3 --dry-run;
  assert_success
  assert_line --index 0 "Found 1 stale databases.";
  assert_line --index 1 "deleting \"$BATS_TEST_TMPDIR/stale.sqlite\" (dry run)";

  assert [ -f "$BATS_TEST_TMPDIR/stale.sqlite" ] # stale db is not removed
}
