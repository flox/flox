#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Ensure schema migrations work as expected.
#
# TODO:
#   - `pkgdb_tables_schema' tests.
#
# ---------------------------------------------------------------------------- #

load setup_suite.bash;

# bats file_tags=sqlite3


# ---------------------------------------------------------------------------- #

setup_file() {
  # We don't parallelize these to avoid DB sync headaches and to recycle the
  # cache between tests.
  # Nonetheless this file makes an effort to avoid depending on past state in
  # such a way that would make it difficult to eventually parallelize in
  # the future.
  export BATS_NO_PARALLELIZE_WITHIN_FILE=true;
  export TEST_HARNESS_FLAKE="$TESTS_DIR/harnesses/proj0";

  export DBPATH="$BATS_FILE_TMPDIR/test.sqlite";

  # Create a new DB.
  $PKGDB scrape --database "$DBPATH" "$TEST_HARNESS_FLAKE"  \
                packages "$NIX_SYSTEM";

}


# ---------------------------------------------------------------------------- #

# get_version ROW-NAME
# --------------------
# Get the version of a row in the DbVersions table.
# Row names:
#   pkgdb
#   pkgdb_tables_schema
#   pkgdb_views_schema
get_version() {
  sqlite3 "$DBPATH" "SELECT version FROM DbVersions WHERE name = '${1?}'";
}


# ---------------------------------------------------------------------------- #

@test "migrate views schema" {
  # Set the version of the views schema to 0, which will always force
  # a migration.
  run sqlite3 "$DBPATH"                                                     \
    "UPDATE DbVersions SET version = 0 WHERE name = 'pkgdb_views_schema'";
  assert_success;

  run get_version pkgdb_views_schema;
  assert_success;
  assert_output 0;

  # Trigger a scrape which should migrate the views schema.
  run $PKGDB scrape --database "$DBPATH" "$TEST_HARNESS_FLAKE"  \
                    packages "$NIX_SYSTEM";
  assert_success;

  # Assert that the views schema was updated.
  run get_version pkgdb_views_schema;
  assert_success;
  refute_output 0;
}


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
