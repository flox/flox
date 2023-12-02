#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# `pkgdb scrape' CLI tests.
#
# These tests target a specific revision of `nixpkgs` and focus on a small
# package set.
#
# ---------------------------------------------------------------------------- #

load setup_suite.bash;

# bats file_tags=cli,scrape,flake


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
}


# ---------------------------------------------------------------------------- #

# This attrset only contains a single package so it's a quick run.
@test "pkgdb scrape <NIXPKGS> legacyPackages $NIX_SYSTEM akkoma-emoji" {
  run $PKGDB scrape --database "$DBPATH" "$NIXPKGS_REF"           \
                    legacyPackages "$NIX_SYSTEM" 'akkoma-emoji';
  assert_success;
}


# ---------------------------------------------------------------------------- #

# Check the description of a package.
@test "akkoma-emoji description" {
  run $PKGDB scrape --database "$DBPATH" "$NIXPKGS_REF"           \
                    legacyPackages "$NIX_SYSTEM" 'akkoma-emoji';
  assert_success;
  local _dID;
  _dID="$(
    sqlite3 "$DBPATH"                                       \
    "SELECT descriptionId FROM Packages  \
     WHERE name = 'blobs.gg-unstable-2019-07-24' LIMIT 1";
  )";
  assert test "$_dID" = 1;
  run sqlite3 "$DBPATH"                                               \
    "SELECT description FROM Descriptions WHERE id = $_dID LIMIT 1";
  assert_output 'Blob emoji from blobs.gg repacked as APNG';
}


# ---------------------------------------------------------------------------- #

# Check the version of a package.
@test "akkoma-emoji version" {
  run $PKGDB scrape --database "$DBPATH" "$NIXPKGS_REF"           \
                    legacyPackages "$NIX_SYSTEM" 'akkoma-emoji';
  assert_success;
  run sqlite3 "$DBPATH" "SELECT version FROM Packages      \
    WHERE name = 'blobs.gg-unstable-2019-07-24' LIMIT 1";
  assert_output 'unstable-2019-07-24';
}


# ---------------------------------------------------------------------------- #

# Check the semver of a package with a non-semantic version.
@test "akkoma-emoji semver" {
  run $PKGDB scrape --database "$DBPATH" "$NIXPKGS_REF"           \
                    legacyPackages "$NIX_SYSTEM" 'akkoma-emoji';
  assert_success;
  run sqlite3 "$DBPATH" "SELECT semver FROM Packages      \
    WHERE name = 'blobs.gg-unstable-2019-07-24' LIMIT 1";
  refute_output --regexp '.';
}


# ---------------------------------------------------------------------------- #

# Check the pname of a package.
@test "akkoma-emoji pname" {
  run $PKGDB scrape --database "$DBPATH" "$NIXPKGS_REF"           \
                    legacyPackages "$NIX_SYSTEM" 'akkoma-emoji';
  assert_success;
  run sqlite3 "$DBPATH" "SELECT pname FROM Packages      \
    WHERE name = 'blobs.gg-unstable-2019-07-24' LIMIT 1";
  assert_output 'blobs.gg';
}


# ---------------------------------------------------------------------------- #

# Check the attribute name of a package
@test "akkoma-emoji attrName" {
  run $PKGDB scrape --database "$DBPATH" "$NIXPKGS_REF"           \
                    legacyPackages "$NIX_SYSTEM" 'akkoma-emoji';
  assert_success;
  run sqlite3 "$DBPATH" "SELECT attrName FROM Packages      \
    WHERE name = 'blobs.gg-unstable-2019-07-24' LIMIT 1";
  assert_output 'blobs_gg';
}


# ---------------------------------------------------------------------------- #

# Check the license of a package
@test "akkoma-emoji license" {
  run $PKGDB scrape --database "$DBPATH" "$NIXPKGS_REF"           \
                    legacyPackages "$NIX_SYSTEM" 'akkoma-emoji';
  assert_success;
  run sqlite3 "$DBPATH" "SELECT license FROM Packages      \
    WHERE name = 'blobs.gg-unstable-2019-07-24' LIMIT 1";
  assert_output 'Apache-2.0';
}


# ---------------------------------------------------------------------------- #

# Check the outputs of a package
@test "akkoma-emoji outputs" {
  run $PKGDB scrape --database "$DBPATH" "$NIXPKGS_REF"           \
                    legacyPackages "$NIX_SYSTEM" 'akkoma-emoji';
  assert_success;
  run sqlite3 "$DBPATH" "SELECT outputs FROM Packages      \
    WHERE name = 'blobs.gg-unstable-2019-07-24' LIMIT 1";
  assert_output '["out"]';
}


# ---------------------------------------------------------------------------- #

# Check the outputs to install from a package
@test "akkoma-emoji outputsToInstall" {
  run $PKGDB scrape --database "$DBPATH" "$NIXPKGS_REF"           \
                    legacyPackages "$NIX_SYSTEM" 'akkoma-emoji';
  assert_success;
  run sqlite3 "$DBPATH" "SELECT outputsToInstall FROM Packages      \
    WHERE name = 'blobs.gg-unstable-2019-07-24' LIMIT 1";
  assert_output '["out"]';
}


# ---------------------------------------------------------------------------- #

# Check whether a package is broken
@test "akkoma-emoji broken" {
  run $PKGDB scrape --database "$DBPATH" "$NIXPKGS_REF"           \
                    legacyPackages "$NIX_SYSTEM" 'akkoma-emoji';
  assert_success;
  run sqlite3 "$DBPATH" "SELECT broken FROM Packages      \
    WHERE name = 'blobs.gg-unstable-2019-07-24' LIMIT 1";
  assert_output '0';
}


# ---------------------------------------------------------------------------- #

# Check whether a package is unfree
@test "akkoma-emoji unfree" {
  run $PKGDB scrape --database "$DBPATH" "$NIXPKGS_REF"           \
                    legacyPackages "$NIX_SYSTEM" 'akkoma-emoji';
  assert_success;
  run sqlite3 "$DBPATH" "SELECT unfree FROM Packages      \
    WHERE name = 'blobs.gg-unstable-2019-07-24' LIMIT 1";
  assert_output '0';
}


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
