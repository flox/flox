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

load setup_suite.bash

# bats file_tags=sqlite3

# ---------------------------------------------------------------------------- #

setup_file() {
  # We don't parallelize these to avoid DB sync headaches and to recycle the
  # cache between tests.
  # Nonetheless this file makes an effort to avoid depending on past state in
  # such a way that would make it difficult to eventually parallelize in
  # the future.
  export BATS_NO_PARALLELIZE_WITHIN_FILE=true
  export TEST_HARNESS_FLAKE="$TESTS_DIR/harnesses/proj0"

  export DBPATH="$BATS_FILE_TMPDIR/test.sqlite"

  # Create a new DB.
  "$PKGDB_BIN" scrape --database "$DBPATH" "$TEST_HARNESS_FLAKE" \
    packages "$NIX_SYSTEM"

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
  sqlite3 "$DBPATH" "SELECT version FROM DbVersions WHERE name = '${1?}'"
}

get_scrape_meta() {
  sqlite3 "$DBPATH" "SELECT value FROM DbScrapeMeta WHERE key = '${1?}'"
}

get_package_count() {
  sqlite3 "$DBPATH" "SELECT COUNT(*) FROM Packages"
}
# ---------------------------------------------------------------------------- #

@test "migrate views schema" {
  # Set the version of the views schema to 0, which will always force
  # a migration.
  run sqlite3 "$DBPATH" \
    "UPDATE DbVersions SET version = 0 WHERE name = 'pkgdb_views_schema'"
  assert_success

  run get_version pkgdb_views_schema
  assert_success
  assert_output 0

  # Trigger a scrape which should migrate the views schema.
  run "$PKGDB_BIN" scrape --database "$DBPATH" "$TEST_HARNESS_FLAKE" \
    packages "$NIX_SYSTEM"
  assert_success

  # Assert that the views schema was updated.
  run get_version pkgdb_views_schema
  assert_success
  refute_output 0
}

@test "invalidation based on rules hash" {
  # Get the package count and hash to start, change the stored hash, 
  # and delete the packages
  
  real_package_count=$(get_package_count)
  assert [ $real_package_count > 0 ]

  # Save current rules hash
  old_rules_hash=$(get_scrape_meta scrape_rules_hash)
  
  # Delete the packages
  run sqlite3 "$DBPATH" "DELETE from Packages"
  assert_success
  assert_equal $(get_package_count) 0
  
  # Trigger a scrape and make sure that alone doesn't re-trigger
  #
  # TODO It does! So this is not a fair test!  We really need to do a search.
  # 
  ##     run "$PKGDB_BIN" scrape --database "$DBPATH" "$TEST_HARNESS_FLAKE" \
  ##       packages "$NIX_SYSTEM"
  ##     assert_success
  ##     assert_equal $(get_package_count) $real_package_count
  
  # Modify and ensure it's different
  run sqlite3 "$DBPATH" \
    "UPDATE DbScrapeMeta SET value = 'md5:invalid' WHERE key = 'scrape_rules_hash'"
  assert_success
  refute [ "$old_rules_hash" == "$(get_scrape_meta scrape_rules_hash)" ]

  # Trigger a scrape which should result in an invalidation *for the next run*
  run "$PKGDB_BIN" scrape --database "$DBPATH" "$TEST_HARNESS_FLAKE" \
    packages "$NIX_SYSTEM"
  assert_success
  
  # Trigger a scrape which should now actually re-scrape
  run "$PKGDB_BIN" scrape --database "$DBPATH" "$TEST_HARNESS_FLAKE" \
    packages "$NIX_SYSTEM"
  assert_success
  new_rules_hash=$(get_scrape_meta scrape_rules_hash)
  assert_equal $old_rules_hash $new_rules_hash
  
  rescraped_package_count=$(get_package_count)
  assert_equal $real_package_count $rescraped_package_count
}

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
