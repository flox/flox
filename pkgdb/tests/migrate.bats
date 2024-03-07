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
  export TEST_DATA="$TESTS_DIR/data/search"

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
  sqlite3 "${1?}" "SELECT value FROM DbScrapeMeta WHERE key = '${2?}'"
}

get_package_count() {
  sqlite3 "${1?}" "SELECT COUNT(*) FROM Packages"
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

genParamsNixpkgsFlox() {
  jq -r ".query.match|=null
          |.manifest.registry.inputs.nixpkgs.from|=\"$TEST_HARNESS_FLAKE\"" \
    "$TEST_DATA/params-local.json"
}

@test "invalidation based on rules hash" {
  # Get the package count and hash to start, change the stored hash, 
  # and delete the packages
  
  search_params="$(genParamsNixpkgsFlox)"
  run sh -c "$PKGDB_BIN search '$search_params'"
  assert_output --partial 'pkg1'
  
  # Find the db path this was scraped to
  dbpath=$($PKGDB_BIN get db $TEST_HARNESS_FLAKE)

  # Make sure we have > 0 packages, save the rules hash
  real_package_count=$(get_package_count $dbpath)
  assert [ $real_package_count > 0 ]
  old_rules_hash=$(get_scrape_meta $dbpath scrape_rules_hash)
  
  # Delete the packages, and assert it's empty
  run sqlite3 "$dbpath" "DELETE from Packages"
  assert_success
  assert_equal $(get_package_count $dbpath) 0
  
  # Search and confirm no packages are found now and we do NOT trigger a re-scrape
  # Do it twice since the first will invalidate the db, and the second will re-create
  run sh -c "$PKGDB_BIN search '$search_params'"
  refute_output --partial 'pkg1'
  run sh -c "$PKGDB_BIN search '$search_params'"
  refute_output --partial 'pkg1'
  
  # Modify the rules hash and ensure it's different
  run sqlite3 "$dbpath" \
    "UPDATE DbScrapeMeta SET value = 'md5:invalid' WHERE key = 'scrape_rules_hash'"
  assert_success
  refute [ "$old_rules_hash" == "$(get_scrape_meta $dbpath scrape_rules_hash)" ]

  # Run a search which should result in an invalidation and re-creation of the
  # database
  run sh -c "$PKGDB_BIN search '$search_params'"
  assert_output --partial 'pkg1'
  new_rules_hash=$(get_scrape_meta $dbpath scrape_rules_hash)
  assert_equal $old_rules_hash $new_rules_hash
  
  rescraped_package_count=$(get_package_count $dbpath)
  assert_equal $real_package_count $rescraped_package_count
}

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
