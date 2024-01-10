#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# `pkgdb search' tests that cannot re-use the database produced by search.bats
#
# ---------------------------------------------------------------------------- #

load setup_suite.bash

# bats file_tags=search

setup_file() {
  export TDATA="$TESTS_DIR/data/search"

  export PKGDB_CACHEDIR="$BATS_FILE_TMPDIR/pkgdbs"
  echo "PKGDB_CACHEDIR: $PKGDB_CACHEDIR" >&3
  # We don't parallelize these to avoid DB sync headaches and to recycle the
  # cache between tests.
  # Nonetheless this file makes an effort to avoid depending on past state in
  # such a way that would make it difficult to eventually parallelize in
  # the future.
  export BATS_NO_PARALLELIZE_WITHIN_FILE=true

  export GA_GLOBAL_MANIFEST="$TESTS_DIR/data/manifest/global-ga0.toml"
}

# Dump parameters for a query on `nixpkgs'.
genParams() {
  jq -r '.query.match|=null' "$TDATA/params0.json" | jq "${1?}"
}

# Dump empty params with a global manifest
genGMParams() {
  # "{\"global-manifest\": \"$GA_GLOBAL_MANIFEST\"}" | jq "${1?}";
  echo '{"global-manifest": "'"$GA_GLOBAL_MANIFEST"'"}' | jq "${1?}"
}

genParamsNixpkgsFlox() {
  jq -r '.query.match|=null
        |.manifest.registry.inputs|=( del( .nixpkgs )|del( .floco ) )' \
    "$TDATA/params1.json" | jq "${1?}"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=search:unfree

# Unfree filter
@test "'pkgdb search' 'allow.unfree=false'" {
  run sh -c "$PKGDB_BIN search '$(
    genParams '.manifest.options.allow.unfree=true'
  )'|wc -l;"
  assert_success

  _count="$output";

  run sh -c "$PKGDB_BIN search '$(
    genParams '.manifest.options.allow.unfree=false'
  )'|wc -l;"
  assert_success

  _count2="$output";

  run expr "$_count2 < $_count"
  assert_success
}

# ---------------------------------------------------------------------------- #

# bats test_tags=search:broken

# Unfree filter
@test "'pkgdb search' 'allow.broken=true'" {
  run sh -c "$PKGDB_BIN search '$(
    genParams '.manifest.options.allow.broken=true'
  )'|wc -l;"
  assert_success

  _count="$output";

  run sh -c "$PKGDB_BIN search '$(
    genParams '.manifest.options.allow.broken=false'
  )'|wc -l;"
  assert_success

  _count2="$output";

  run expr "$_count2 < $_count"
  assert_success
}

# ---------------------------------------------------------------------------- #

# bats test_tags=search:prerelease, search:pname

# preferPreReleases ordering
@test "'pkgdb search' 'manifest.options.semver.prefer-pre-releases=true'" {
  run sh -c "$PKGDB_BIN search '$(
    genParams '.manifest.options.semver["prefer-pre-releases"]=true
               |.query.pname="zfs-kernel"'
  )'|head -n1|jq -r .version;"
  assert_success
  assert_output '2.1.12-staging-2023-04-18-6.1.31'
}


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
