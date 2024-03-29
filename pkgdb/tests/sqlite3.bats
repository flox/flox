#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# `is_sqlite3' executable tests.
#
# ---------------------------------------------------------------------------- #

load setup_suite.bash

# bats file_tags=sqlite3

# ---------------------------------------------------------------------------- #

setup_file() {
  export DBPATH="$BATS_FILE_TMPDIR/test.sqlite"

  # Make test files
  mkdir -p "$BATS_FILE_TMPDIR/dir"
  # Fewer than 16 chars
  echo 'txt' > "$BATS_FILE_TMPDIR/short"
  # More than 16 chars
  echo '0123456789012345679' > "$BATS_FILE_TMPDIR/long"
  # Make a test DB
  sqlite3 "$DBPATH" 'CREATE TABLE People ( name TEXT PRIMARY KEY )'

  if [[ -z "${PKGDB_IS_SQLITE3_BIN:-}" ]]; then
    repo_root_setup
    PKGDB_IS_SQLITE3_BIN="$TESTS_DIR/is_sqlite3"
    if ! [[ -x "$PKGDB_IS_SQLITE3_BIN" ]]; then
      (
        cd "${REPO_ROOT?}" > /dev/null 2>&1 || exit 1
        make -j tests/is_sqlite3
      )
    fi
  fi
  export PKGDB_IS_SQLITE3_BIN
}

# ---------------------------------------------------------------------------- #

@test "is_sqlite3 detects DB" {
  run "$PKGDB_IS_SQLITE3_BIN" "$DBPATH"
  assert_success
}

# ---------------------------------------------------------------------------- #

@test "is_sqlite3 rejects text file ( short )" {
  run "$PKGDB_IS_SQLITE3_BIN" "$BATS_FILE_TMPDIR/short"
  assert_failure
}

# ---------------------------------------------------------------------------- #

@test "is_sqlite3 rejects text file ( long )" {
  run "$PKGDB_IS_SQLITE3_BIN" "$BATS_FILE_TMPDIR/long"
  assert_failure
}

# ---------------------------------------------------------------------------- #

@test "is_sqlite3 rejects directory" {
  run "$PKGDB_IS_SQLITE3_BIN" "$BATS_FILE_TMPDIR/dir"
  assert_failure
}

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
