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

# bats file_tags=cli,get


# ---------------------------------------------------------------------------- #

setup_file() {
  export DBPATH="$BATS_FILE_TMPDIR/test.sqlite";
  mkdir -p "$BATS_FILE_TMPDIR";
  if $PKGDB scrape --database "$DBPATH" "$NIXPKGS_REF"           \
                   legacyPackages "$NIX_SYSTEM" 'akkoma-emoji';
  then
    echo "Scraped flake $NIXPKGS_REF" >&3;
    export SKIP_SCRAPED=;
  else
    echo "Failed to scrape flake $NIXPKGS_REF" >&3;
    echo "Some tests will be skipped" >&3;
    export SKIP_SCRAPED=:;
  fi
}


# ---------------------------------------------------------------------------- #

require_shared() {
  if test -n "${SKIP_SCRAPED:-}"; then
    skip "This test requires \`pkgdb scrape', but a failure was encountered.";
  fi
}


# ---------------------------------------------------------------------------- #

# bats test_tags=get:id

# The root of a flake always has `row_id' 0.
@test "pkgdb get id <EMPTY>" {
  require_shared;
  run $PKGDB get id "$DBPATH";
  assert_success;
  assert_output '0';
}


# ---------------------------------------------------------------------------- #

# bats test_tags=get:id

# Checking for a non-existent path should exit fail.
@test "pkgdb get id <NON-EXISTENT>" {
  require_shared;
  run $PKGDB get id "$DBPATH" phony;
  assert_failure;
}


# ---------------------------------------------------------------------------- #

# bats test_tags=get:id

# Since we only scraped one prefix, we know `legacyPackages` has `row_id` 1.
@test "pkgdb get id legacyPackages" {
  require_shared;
  run $PKGDB get id "$DBPATH" legacyPackages;
  assert_success;
  assert_output 1;
}


# ---------------------------------------------------------------------------- #

# bats test_tags=get:id

# Another known `row_id'.
@test "pkgdb get id legacyPackages $NIX_SYSTEM" {
  require_shared;
  run $PKGDB get id "$DBPATH" legacyPackages "$NIX_SYSTEM";
  assert_success;
  assert_output 2;
}


# ---------------------------------------------------------------------------- #

# bats test_tags=get:id

# Another known `row_id'.
@test "pkgdb get id legacyPackages $NIX_SYSTEM akkoma-emoji" {
  require_shared;
  run $PKGDB get id "$DBPATH" legacyPackages "$NIX_SYSTEM" 'akkoma-emoji';
  assert_success;
  assert_output 3;
}


# ---------------------------------------------------------------------------- #

# bats test_tags=get:path

# Expect empty list for "root".
@test "pkgdb get path 0" {
  require_shared;
  run $PKGDB get path "$DBPATH" 0;
  assert_success;
  assert_output '[]';
}


# ---------------------------------------------------------------------------- #

# bats test_tags=get:path

# This path is already known.
@test "pkgdb get path 1" {
  require_shared;
  run $PKGDB get path "$DBPATH" 1;
  assert_success;
  assert_output '["legacyPackages"]';
}


# ---------------------------------------------------------------------------- #

# bats test_tags=get:path

# This path is already known.
@test "pkgdb get path 2" {
  require_shared;
  run $PKGDB get path "$DBPATH" 2;
  assert_success;
  assert_output "[\"legacyPackages\",\"$NIX_SYSTEM\"]";
}


# ---------------------------------------------------------------------------- #

# bats test_tags=get:path

# This path is already known.
@test "pkgdb get path 3" {
  require_shared;
  run $PKGDB get path "$DBPATH" 3;
  assert_success;
  assert_output "[\"legacyPackages\",\"$NIX_SYSTEM\",\"akkoma-emoji\"]";
}


# ---------------------------------------------------------------------------- #

# bats test_tags=get:path

# Expect failure for a non-existent `row_id'.
@test "pkgdb get path <NON-EXISTENT>" {
  require_shared;
  run $PKGDB get path "$DBPATH" 999;
  assert_failure;
}


# ---------------------------------------------------------------------------- #

# bats test_tags=get:path

# We only have a single package so we know its path and `row_id'.
@test "pkgdb get path --pkg 1" {
  require_shared;
  run $PKGDB get path --pkg "$DBPATH" 1;
  assert_success;
  assert_output                                                            \
    "[\"legacyPackages\",\"$NIX_SYSTEM\",\"akkoma-emoji\",\"blobs_gg\"]";
}


# ---------------------------------------------------------------------------- #

# bats test_tags=get:id

# We only have a single package so we know its path and `row_id'.
@test "pkgdb get id --pkg legacyPackages $NIX_SYSTEM akkoma-emoji blobs_gg" {
  require_shared;
  run $PKGDB get id --pkg "$DBPATH"                                      \
                    legacyPackages "$NIX_SYSTEM" akkoma-emoji blobs_gg;
  assert_success;
  assert_output 1;
}


# ---------------------------------------------------------------------------- #

# bats test_tags=get:id

# Non-package attribute path should fail.
@test "pkgdb get id --pkg legacyPackages $NIX_SYSTEM akkoma-emoji" {
  require_shared;
  run $PKGDB get id --pkg "$DBPATH"                             \
                    legacyPackages "$NIX_SYSTEM" akkoma-emoji;
  assert_failure;
}


# ---------------------------------------------------------------------------- #

# bats test_tags=get:flake

@test "pkgdb get flake" {
  require_shared;
  run sh -c "$PKGDB get flake '$DBPATH'|jq -r '.string';";
  assert_success;
  assert_output "$NIXPKGS_REF";
  run sh -c "$PKGDB get flake '$DBPATH'|jq -r '.attrs.rev';";
  assert_success;
  assert_output "${NIXPKGS_REF##*/}";
  run sh -c "$PKGDB get flake '$DBPATH'|jq -r '.fingerprint';";
  assert_success;
  assert_output "$NIXPKGS_FINGERPRINT";
}


# ---------------------------------------------------------------------------- #

# bats test_tags=get:db

# When given a path to a db we should parrot the input we were given.
# This one is kind of nonsensical but it is a side effect of argument processing
# that's worth preserving.
#
# Later changes should feel free to break this test and change it, but when
# doing so be sure to thoroughly perform integration testing with users
# of `pkgdb`, since they may rely on this seemingly useless behavior.
@test "pkgdb get db <DB-PATH>" {
  require_shared;
  run $PKGDB get db "$DBPATH";
  assert_success;
  assert_output "$DBPATH";
}


# ---------------------------------------------------------------------------- #

# bats test_tags=get:db

@test "pkgdb get db <FLAKE-REF>" {
  require_shared;
  run $PKGDB get db "$NIXPKGS_REF";
  assert_success;
  assert_output --partial "/$NIXPKGS_FINGERPRINT.sqlite";
}


# ---------------------------------------------------------------------------- #

# bats test_tags=get:done

@test "pkgdb get done <DB-PATH> legacyPackages $NIX_SYSTEM akkoma-emoji" {
  require_shared;
  run $PKGDB get 'done' "$DBPATH" legacyPackages "$NIX_SYSTEM" akkoma-emoji;
  assert_success;
}


@test "pkgdb get done <DB-PATH> legacyPackages $NIX_SYSTEM" {
  require_shared;
  run $PKGDB get 'done' "$DBPATH" legacyPackages "$NIX_SYSTEM";
  assert_failure;
}


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
