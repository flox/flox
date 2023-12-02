#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# `pkgdb scrape' tests focused on a local flake with intentionally
# "evil" metadata.
#
# This largely aims to test edge case detection.
#
#
# ---------------------------------------------------------------------------- #

load setup_suite.bash;

# bats file_tags=cli,scrape,flake,local,legacy


# ---------------------------------------------------------------------------- #

setup_file() {
  export DBPATH="$BATS_FILE_TMPDIR/test.sqlite";
  mkdir -p "$BATS_FILE_TMPDIR";
  # We don't parallelize these to avoid DB sync headaches and to recycle the
  # cache between tests.
  # Nonetheless this file makes an effort to avoid depending on past state in
  # such a way that would make it difficult to eventually parallelize in
  # the future.
  export BATS_NO_PARALLELIZE_WITHIN_FILE=true;
  export TEST_HARNESS_FLAKE="$TESTS_DIR/harnesses/proj0";
}

# ---------------------------------------------------------------------------- #

@test "ugly packages are scraped" {
  run $PKGDB scrape --database "$DBPATH" "$TEST_HARNESS_FLAKE"  \
                    packages "$NIX_SYSTEM";
  assert_success;
  run sqlite3 "$DBPATH" "SELECT COUNT(*) from Packages";
  assert_output '6'; # pkgs0-pkgs4 + default
}


# ---------------------------------------------------------------------------- #

@test "pkg0 has no 'version' attr" {
  run $PKGDB scrape --database "$DBPATH" "$TEST_HARNESS_FLAKE"  \
                    packages "$NIX_SYSTEM";
  assert_success;
  run sqlite3 "$DBPATH" "SELECT version FROM Packages  \
    WHERE attrName = 'pkg0' LIMIT 1";
  refute_output --regexp '.';
}

# ---------------------------------------------------------------------------- #

@test "pkg0 has no 'description' attr" {
  run $PKGDB scrape --database "$DBPATH" "$TEST_HARNESS_FLAKE"  \
                    packages "$NIX_SYSTEM";
  assert_success;
  run sqlite3 "$DBPATH" "SELECT descriptionId FROM Packages  \
    WHERE attrName = 'pkg0' LIMIT 1";
  refute_output --regexp '.';
}


# ---------------------------------------------------------------------------- #

@test "pkg1 'name' is constructed" {
  run $PKGDB scrape --database "$DBPATH" "$TEST_HARNESS_FLAKE"  \
                    packages "$NIX_SYSTEM";
  assert_success;
  run sqlite3 "$DBPATH" "SELECT name FROM Packages  \
    WHERE attrName = 'pkg1' LIMIT 1";
  assert_output 'pkg-1';
}


# ---------------------------------------------------------------------------- #

@test "pkg1 'version' translated to 'semver'" {
  run $PKGDB scrape --database "$DBPATH" "$TEST_HARNESS_FLAKE"  \
                    packages "$NIX_SYSTEM";
  assert_success;
  run sqlite3 "$DBPATH" "SELECT semver FROM Packages  \
    WHERE attrName = 'pkg1' LIMIT 1";
  assert_output '1.0.0';
}


# ---------------------------------------------------------------------------- #

@test "pkg2 'pname' extracted" {
  run $PKGDB scrape --database "$DBPATH" "$TEST_HARNESS_FLAKE"  \
                    packages "$NIX_SYSTEM";
  assert_success;
  run sqlite3 "$DBPATH" "SELECT pname FROM Packages      \
    WHERE attrName = 'pkg2' LIMIT 1";
  assert_output 'pkg';
}


# ---------------------------------------------------------------------------- #

@test "pkg2 'version' extracted" {
  run $PKGDB scrape --database "$DBPATH" "$TEST_HARNESS_FLAKE"  \
                    packages "$NIX_SYSTEM";
  assert_success;
  run sqlite3 "$DBPATH" "SELECT version FROM Packages  \
    WHERE attrName = 'pkg2' LIMIT 1";
  assert_output '2';
}


# ---------------------------------------------------------------------------- #

@test "pkg2 'version' translated to 'semver'" {
  run $PKGDB scrape --database "$DBPATH" "$TEST_HARNESS_FLAKE"  \
                    packages "$NIX_SYSTEM";
  assert_success;
  run sqlite3 "$DBPATH" "SELECT semver FROM Packages  \
    WHERE attrName = 'pkg2' LIMIT 1";
  assert_output '2.0.0';
}


# ---------------------------------------------------------------------------- #

@test "pkg2 'license' ignored" {
  run $PKGDB scrape --database "$DBPATH" "$TEST_HARNESS_FLAKE"  \
                    packages "$NIX_SYSTEM";
  assert_success;
  run sqlite3 "$DBPATH" "SELECT license FROM Packages  \
    WHERE attrName = 'pkg2' LIMIT 1";
  refute_output --regexp '.';
}


# ---------------------------------------------------------------------------- #

@test "pkg2 'unfree' set with bad license" {
  run $PKGDB scrape --database "$DBPATH" "$TEST_HARNESS_FLAKE"  \
                    packages "$NIX_SYSTEM";
  assert_success;
  run sqlite3 "$DBPATH" "SELECT unfree FROM Packages  \
    WHERE attrName = 'pkg2' LIMIT 1";
  assert_output '0';
}


# ---------------------------------------------------------------------------- #

@test "pkg3 'name' constructed" {
  run $PKGDB scrape --database "$DBPATH" "$TEST_HARNESS_FLAKE"  \
                    packages "$NIX_SYSTEM";
  assert_success;
  run sqlite3 "$DBPATH" "SELECT name FROM Packages  \
    WHERE name = 'pkg-2023-08-09' LIMIT 1";
  assert_output 'pkg-2023-08-09';
}


# ---------------------------------------------------------------------------- #

@test "pkg3 has no 'semver' attr" {
  run $PKGDB scrape --database "$DBPATH" "$TEST_HARNESS_FLAKE"  \
                    packages "$NIX_SYSTEM";
  assert_success;
  run sqlite3 "$DBPATH" "SELECT semver FROM Packages  \
    WHERE attrName = 'pkg3' LIMIT 1";
  refute_output --regexp '.';
}


# ---------------------------------------------------------------------------- #

@test "pkg4 'name' == 'pname'" {
  run $PKGDB scrape --database "$DBPATH" "$TEST_HARNESS_FLAKE"  \
                    packages "$NIX_SYSTEM";
  assert_success;
  run sqlite3 "$DBPATH" "SELECT pname FROM Packages  \
    WHERE attrName = 'pkg4' LIMIT 1";
  assert_output 'pkg';
}


# ---------------------------------------------------------------------------- #

@test "pkg4 has no 'broken' attr" {
  run $PKGDB scrape --database "$DBPATH" "$TEST_HARNESS_FLAKE"  \
                    packages "$NIX_SYSTEM";
  assert_success;
  run sqlite3 "$DBPATH" "SELECT broken FROM Packages  \
    WHERE attrName = 'pkg4' LIMIT 1";
  refute_output --regexp '.';
}


# ---------------------------------------------------------------------------- #

@test "pkg4 has no 'unfree' attr" {
  run $PKGDB scrape --database "$DBPATH" "$TEST_HARNESS_FLAKE"  \
                    packages "$NIX_SYSTEM";
  assert_success;
  run sqlite3 "$DBPATH" "SELECT unfree FROM Packages  \
    WHERE attrName = 'pkg4' LIMIT 1";
  refute_output --regexp '.';
}


# ---------------------------------------------------------------------------- #

@test "pkg4 has no 'license' attr" {
  run $PKGDB scrape --database "$DBPATH" "$TEST_HARNESS_FLAKE"  \
                    packages "$NIX_SYSTEM";
  assert_success;
  run sqlite3 "$DBPATH" "SELECT license FROM Packages  \
    WHERE attrName = 'pkg4' LIMIT 1";
  refute_output --regexp '.';
}


# ---------------------------------------------------------------------------- #

@test "pkg4 has no 'version' attr" {
  run $PKGDB scrape --database "$DBPATH" "$TEST_HARNESS_FLAKE"  \
                    packages "$NIX_SYSTEM";
  assert_success;
  run sqlite3 "$DBPATH" "SELECT version FROM Packages  \
    WHERE attrName = 'pkg4' LIMIT 1";
  refute_output --regexp '.';
}


# ---------------------------------------------------------------------------- #

@test "pkg4 has no 'semver' attr" {
  run $PKGDB scrape --database "$DBPATH" "$TEST_HARNESS_FLAKE"  \
                    packages "$NIX_SYSTEM";
  assert_success;
  run sqlite3 "$DBPATH" "SELECT semver FROM Packages  \
    WHERE attrName = 'pkg4' LIMIT 1";
  refute_output --regexp '.';
}


# ---------------------------------------------------------------------------- #

@test "pkg4 has no 'descriptionId'" {
  run $PKGDB scrape --database "$DBPATH" "$TEST_HARNESS_FLAKE"  \
                    packages "$NIX_SYSTEM";
  assert_success;
  run sqlite3 "$DBPATH" "SELECT descriptionId FROM Packages  \
    WHERE attrName = 'pkg4' LIMIT 1";
  refute_output --regexp '.';
}


# ---------------------------------------------------------------------------- #

@test "default package has no 'version' attr" {
  run $PKGDB scrape --database "$DBPATH" "$TEST_HARNESS_FLAKE"  \
                    packages "$NIX_SYSTEM";
  assert_success;
  run sqlite3 "$DBPATH" "SELECT version FROM Packages  \
    WHERE attrName = 'default' LIMIT 1";
  refute_output --regexp '.';
}


# ---------------------------------------------------------------------------- #

@test "default package has no 'description' attr" {
  run $PKGDB scrape --database "$DBPATH" "$TEST_HARNESS_FLAKE"  \
                    packages "$NIX_SYSTEM";
  assert_success;
  run sqlite3 "$DBPATH" "SELECT descriptionId FROM Packages  \
    WHERE attrName = 'default' LIMIT 1";
  refute_output --regexp '.';
}


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
