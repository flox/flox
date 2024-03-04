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

# bats test_tags=search:match

# Searches `nixpkgs#legacyPackages.x86_64-linux' for a fuzzy match on "hello"
@test "'pkgdb search' params0.json" {
  run "$PKGDB_BIN" search "$TDATA/params0.json"
  assert_success
}

# ---------------------------------------------------------------------------- #

# bats test_tags=search:opt

@test "'pkgdb search' scrapes only named subtrees" {
  DBPATH="$($PKGDB_BIN get db "$NIXPKGS_REF")"
  run "$PKGDB_BIN" search "$TDATA/params0.json"
  assert_success
  run "$PKGDB_BIN" get id "$DBPATH" x86_64-linux packages
  assert_failure
}

# ---------------------------------------------------------------------------- #

# bats test_tags=search:match
#
@test "'pkgdb search' 'match=hello'" {
  run sh -c "$PKGDB_BIN search '$TDATA/params0.json' | wc -l"
  assert_success
  original_count="$output"
  run sh -c "$PKGDB_BIN search '$TDATA/params0.json' | grep hello | wc -l"
  assert_success
  assert_output "$original_count"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=search:pname

# Exact `pname' match
@test "'pkgdb search' 'pname=hello'" {
  params="$(genParams '.query.pname|="hello"')"
  run sh -c "$PKGDB_BIN search '$params' | wc -l"
  assert_success
  assert_output 1
}

# ---------------------------------------------------------------------------- #

# bats test_tags=search:version, search:pname

# Exact `version' match
@test "'pkgdb search' 'pname=nodejs & version=$NODEJS_VERSION'" {
  params="$(genParams ".query.pname|=\"nodejs\"|.query.version=\"$NODEJS_VERSION\"")"
  run sh -c "$PKGDB_BIN search '$params' | wc -l"
  assert_success
  assert_output 4
}

# ---------------------------------------------------------------------------- #

# bats test_tags=search:semver, search:pname

# Test `semver' by filtering to >$NODEJS_VERSION, leaving 20.9.0, 21.2.0, and an alias of 21.2.0
@test "'pkgdb search' 'pname=nodejs & semver=>$NODEJS_VERSION'" {
  params="$(genParams ".query.pname|=\"nodejs\"|.query.semver=\">$NODEJS_VERSION\"")"
  $PKGDB_BIN search "$params"
  run sh -c "$PKGDB_BIN search '$params' | wc -l"
  assert_success
  assert_output 3
}

# ---------------------------------------------------------------------------- #

# bats test_tags=search:semver, search:pname

@test "'pkgdb search' with partial semvers (such as those in an .nvmrc)" {
  params="$(genParams ".query.pname|=\"nodejs\"|.query.semver=\"18\"")"
  $PKGDB_BIN search "$params"
  run sh -c "$PKGDB_BIN search '$params' | wc -l"
  assert_success
  assert_output 4

  params="$(genParams ".query.pname|=\"nodejs\"|.query.semver=\"18.18\"")"
  $PKGDB_BIN search "$params"
  run sh -c "$PKGDB_BIN search '$params' | wc -l"
  assert_success
  assert_output 4
}

# ---------------------------------------------------------------------------- #

# bats test_tags=search:semver, search:pname

# Test `semver' by filtering to 18.*
@test "'pkgdb search' 'pname=nodejs & semver=18.*'" {
  params="$(genParams '.query.pname|="nodejs"|.query.semver="18.*"')"
  run sh -c "$PKGDB_BIN search '$params' | wc -l"
  assert_success
  assert_output 4
}

# ---------------------------------------------------------------------------- #

# bats test_tags=search:name

# Exact `name' match.
@test "'pkgdb search' name=nodejs-$NODEJS_VERSION" {
  params="$(genParams ".query.name|=\"nodejs-$NODEJS_VERSION\"")"
  run sh -c "$PKGDB_BIN search '$params' | wc -l;"
  assert_success
  assert_output 4
}

# ---------------------------------------------------------------------------- #

# bats test_tags=search:name, search:license

# Licenses filter
@test "'pkgdb search' 'pname=blobs.gg & licenses=[Apache-2.0]'" {
  params="$(
    genParams '.query.pname|="blobs.gg"
              |.manifest.options.allow.licenses|=["Apache-2.0"]'
  )"
  run sh -c "$PKGDB_BIN search '$params' | wc -l"
  assert_success
  assert_output 1
}

# ---------------------------------------------------------------------------- #

# bats test_tags=search

# Check output fields.
@test "'pkgdb search' emits expected fields" {

  params="$(
    genParams '.manifest.options.systems=["x86_64-linux","x86_64-darwin"]
               |.query.pname="hello"'
  )"
  run sh -c "$PKGDB_BIN search '$params' | head -n1 | jq -r 'to_entries|map( .key + \" \" + ( .value|type ) )[]'"
  assert_success
  assert_output --partial 'absPath array'
  assert_output --partial 'broken boolean'
  assert_output --partial 'description string'
  assert_output --partial 'id number'
  assert_output --partial 'input string'
  assert_output --partial 'license string'
  assert_output --partial 'relPath array'
  assert_output --partial 'pname string'
  assert_output --partial 'subtree string'
  assert_output --partial 'system string'
  assert_output --partial 'unfree boolean'
  assert_output --partial 'version string'
}

# ---------------------------------------------------------------------------- #

# bats test_tags=search:unfree

# Unfree filter
@test "'pkgdb search' 'allow.unfree=false'" {
  params_true="$(genParams '.manifest.options.allow.unfree=true')"
  run sh -c "$PKGDB_BIN search '$params_true' | wc -l"
  assert_success

  _count="$output"

  params_false="$(genParams '.manifest.options.allow.unfree=false')"
  run sh -c "$PKGDB_BIN search '$params_false' | wc -l"
  assert_success

  _count2="$output"

  run expr "$_count2 < $_count"
  assert_success
}

# ---------------------------------------------------------------------------- #

# bats test_tags=search:broken

# Unfree filter
@test "'pkgdb search' 'allow.broken=true'" {
  params_true="$(genParams '.manifest.options.allow.broken=true')"

  run sh -c "$PKGDB_BIN search '$params_true' | wc -l"
  assert_success

  _count="$output"

  params_false=$(genParams '.manifest.options.allow.broken=false')
  run sh -c "$PKGDB_BIN search '$params_false' | wc -l"
  assert_success

  _count2="$output"

  run expr "$_count2 < $_count"
  assert_success
}

# ---------------------------------------------------------------------------- #

# bats test_tags=search:prerelease, search:pname

# setting prefer-pre-releases returns a newer package version that is a
# prerelease
@test "'pkgdb search' 'manifest.options.semver.prefer-pre-releases=true'" {
  params="$(genParams '.query.pname="linux"')"
  run sh -c "$PKGDB_BIN search '$params'|head -n1|jq -r .version;"
  assert_success
  assert_output '6.6.8'

  params="$(genParams '.manifest.options.semver["prefer-pre-releases"]=true
                      |.query.pname="linux"'
  )"
  run sh -c "$PKGDB_BIN search '$params'|head -n1|jq -r .version;"
  assert_success
  assert_output '6.7-rc7'
}

# ---------------------------------------------------------------------------- #

# bats test_tags=search:system, search:pname

# `systems' ordering
@test "'pkgdb search' systems order" {
  params="$(
    genParams '.manifest.options.systems=["x86_64-linux","x86_64-darwin"]
               |.query.pname="hello"'
  )"

  run sh -c "$PKGDB_BIN search '$params' \
             | jq -rs 'to_entries
                      | map(( .key|tostring ) + \" \" + .value.absPath[1])[]'"
  assert_success
  assert_output --partial '0 x86_64-linux'
  assert_output --partial '1 x86_64-darwin'
  refute_output --partial '2 '
}

# bats test_tags=search:system, search:pname

# `systems' ordering, reverse order of previous
@test "'pkgdb search' systems order ( reversed )" {
  params="$(
    genParams '.manifest.options.systems=["x86_64-darwin","x86_64-linux"]
               |.query.pname="hello"'
  )"

  run sh -c "$PKGDB_BIN search '$params' \
             | jq -rs 'to_entries
                      | map(( .key|tostring ) + \" \" + .value.absPath[1])[]'"
  assert_success
  assert_output --partial '0 x86_64-darwin'
  assert_output --partial '1 x86_64-linux'
  refute_output --partial '2 '
}

# ---------------------------------------------------------------------------- #

# bats test_tags=search:params, search:params:fallbacks

# Check fallback behavior.
@test "search-params with empty object" {
  if [ -z "${PKGDB_SEARCH_PARAMS_BIN:=$(command -v search-params)}" ]; then
    skip "Unable to locate \`search-params' binary"
  fi
  run "${PKGDB_SEARCH_PARAMS_BIN:?}" '{}'
  assert_success

  run sh -c "$PKGDB_SEARCH_PARAMS_BIN '{}' | jq -r '.manifest'"
  assert_success
  assert_output 'null'

  run sh -c "$PKGDB_SEARCH_PARAMS_BIN '{}' | jq -r '.query.name'"
  assert_success
  assert_output 'null'

  run sh -c "$PKGDB_SEARCH_PARAMS_BIN '{}' | jq -r '.query.pname'"
  assert_success
  assert_output 'null'

  run sh -c "$PKGDB_SEARCH_PARAMS_BIN '{}' | jq -r '.query.version'"
  assert_success
  assert_output 'null'

  run sh -c "$PKGDB_SEARCH_PARAMS_BIN '{}' | jq -r '.query.semver'"
  assert_success
  assert_output 'null'

  run sh -c "$PKGDB_SEARCH_PARAMS_BIN '{}' | jq -r '.query.match'"
  assert_success
  assert_output 'null'

  run sh -c "$PKGDB_SEARCH_PARAMS_BIN '{}' | jq -r '.query[\"match-name\"]'"
  assert_success
  assert_output 'null'

  run sh -c "$PKGDB_SEARCH_PARAMS_BIN '{}' | jq -r '.[\"global-manifest\"]'"
  assert_success
  assert_output 'null'

  run sh -c "$PKGDB_SEARCH_PARAMS_BIN '{}' | jq -r '.lockfile'"
  assert_success
  assert_output 'null'

}

# ---------------------------------------------------------------------------- #

# bats tests_tags=search:error

@test "'pkgdb search' JSON error when no query present" {
  skip "FIXME: empty search should return no results"
  params="$(genGMParams '.query=null')"
  run "$PKGDB_BIN" search --ga-registry "$params"
  # output depends on resolution of pkgdb#177
}

# ---------------------------------------------------------------------------- #

# bats test_tags=search:error, search:manifest

@test "'pkgdb search' JSON error when no manifests are provided" {
  skip "FIXME: search requires a global manifest, but succeeds without one"
  run "$PKGDB_BIN" search --ga-registry '{"query": {"match": "ripgrep"}}'
  assert_failure
  assert_output --partial "foo"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=search:error, search:manifest

@test "'pkgdb search' JSON error when manifest path does not exist" {
  params="$(genGMParams '.manifest="/does/not/exist"')"
  run "$PKGDB_BIN" search --ga-registry "$params"
  assert_failure
  category_msg="$(echo "$output" | jq '.category_message')"
  context_msg="$(echo "$output" | jq '.context_message')"
  assert_equal "$category_msg" '"invalid manifest file"'
  assert_equal "$context_msg" '"no such path: /does/not/exist"'
}

# bats test_tags=search:error, search:manifest, search:global-manifest

@test "'pkgdb search' JSON error when global manifest path does not exist" {
  params="$(genGMParams '.["global-manifest"]="/does/not/exist"')"
  run "$PKGDB_BIN" search --ga-registry "$params"
  assert_failure
  category_msg="$(echo "$output" | jq '.category_message')"
  context_msg="$(echo "$output" | jq '.context_message')"
  assert_equal "$category_msg" '"invalid manifest file"'
  assert_equal "$context_msg" '"no such path: /does/not/exist"'
}

# bats test_tags=search:error, search:lockfile

@test "'pkgdb search' JSON error when lockfile path does not exist" {
  params="$(genGMParams '.lockfile="/does/not/exist"')"
  run "$PKGDB_BIN" search --ga-registry "$params"
  assert_failure
  category_msg="$(echo "$output" | jq '.category_message')"
  context_msg="$(echo "$output" | jq '.context_message')"
  assert_equal "$category_msg" '"invalid lockfile"'
  assert_equal "$context_msg" '"no such path: /does/not/exist"'
}

# ---------------------------------------------------------------------------- #

# bats tests_tags=search:error

@test "'pkgdb search' JSON error with unexpected query field" {
  params="$(genGMParams '.query.foo="bar"')"
  run "$PKGDB_BIN" search --ga-registry "$params"
  assert_failure
  category_msg="$(echo "$output" | jq '.category_message')"
  context_msg="$(echo "$output" | jq -r '.context_message')"
  assert_equal "$category_msg" '"error parsing search query"'
  assert_equal "$context_msg" "unrecognized key 'query.foo'."
}

# ---------------------------------------------------------------------------- #

# bats tests_tags=search:error, search:lockfile

@test "'pkgdb search' JSON error when lockfile has invalid format" {
  params="$(genGMParams '.query.match="ripgrep"|.lockfile={"foo": "bar"}')"
  run "$PKGDB_BIN" search --ga-registry "$params"
  assert_failure
  category_msg="$(echo "$output" | jq '.category_message')"
  context_msg="$(echo "$output" | jq -r '.context_message')"
  assert_equal "$category_msg" '"invalid lockfile"'
  assert_equal "$context_msg" "encountered unexpected field 'foo' while parsing locked package"
}

# ---------------------------------------------------------------------------- #

# bats tests_tags=search:error

@test "'pkgdb search' JSON error when params not valid JSON" {
  skip "FIXME: need better error message when search params not valid JSON"
  params='{'
  run "$PKGDB_BIN" search --ga-registry "$params"
  assert_failure
  # exact output depends on resolution of pkgdb#184
}

# ---------------------------------------------------------------------------- #

# bats tests_tags=search:error, search:registry

@test "'pkgdb search' no indirect flake references" {
  skip "FIXME: need better error message when indirect flakeref found"
  params="$(jq -c '.' "$TDATA/params2.json")"
  run "$PKGDB_BIN" search "$params"
  assert_failure
  # exact output depends on resolution of pkgdb#183
}

# ---------------------------------------------------------------------------- #

# bats tests_tags=search:error, search:registry

@test "'pkgdb search' JSON error when input does not exist" {
  params="$(jq -c '.' "$TDATA/params3.json")"
  run "$PKGDB_BIN" search -q "$params"
  assert_failure
  category_msg="$(echo "$output" | jq '.category_message')"
  context_msg="$(echo "$output" | jq -r '.context_message')"
  caught_msg="$(echo "$output" | jq -r '.caught_message')"
  assert_equal "$category_msg" '"error locking flake"'
  assert_equal "$context_msg" 'failed to lock flake "github:flox/badrepo"'
  # The caught Nix error is big, only check the beginning
  assert_regex "$caught_msg" "^error:"
}

# ---------------------------------------------------------------------------- #

# bats tests_tags=search

@test "'pkgdb is properly scraping nodePackages" {
  params="$(genGMParams ".query.pname=\"npm\" | .manifest.options.systems=[\"x86_64-linux\"]")"
  run --separate-stderr "$PKGDB_BIN" search -q --ga-registry "$params"
  assert_success
  assert_equal "${#lines[@]}" 2
}

@test "'pkgdb search' with ' in search term" {
  skip "FIXME: no results"
  params="$(genGMParams ".query.match=\"hello'\" | .manifest.options.systems=[\"x86_64-linux\"]")"
  run --separate-stderr "$PKGDB_BIN" search -q --ga-registry "$params"
  assert_success
  assert_equal "${#lines[@]}" 11
}

# bats tests_tags=search

@test "'pkgdb search' with '\"' in search term" {
  skip "FIXME: no results"
  params="$(genGMParams '.query.match="hello"" | .manifest.options.systems=["x86_64-linux"]')"
  run --separate-stderr "$PKGDB_BIN" search -q --ga-registry "$params"
  assert_success
  assert_equal "${#lines[@]}" 11
}

# bats tests_tags=search

@test "'pkgdb search' with '[' in search term" {
  skip "FIXME: no results"
  params="$(genGMParams '.query.match="hello[" | .manifest.options.systems=["x86_64-linux"]')"
  run --separate-stderr "$PKGDB_BIN" search -q --ga-registry "$params"
  assert_success
  assert_equal "${#lines[@]}" 11
}

# bats tests_tags=search

@test "'pkgdb search' with ']' in search term" {
  skip "FIXME: no results"
  params="$(genGMParams '.query.match="hello]" | .manifest.options.systems=["x86_64-linux"]')"
  run --separate-stderr "$PKGDB_BIN" search -q --ga-registry "$params"
  assert_success
  assert_equal "${#lines[@]}" 0
}

# bats tests_tags=search

@test "'pkgdb search' with '*' in search term" {
  skip "FIXME: no results"
  params="$(genGMParams '.query.match="hello*" | .manifest.options.systems=["x86_64-linux"]')"
  run --separate-stderr "$PKGDB_BIN" search -q --ga-registry "$params"
  assert_success
  assert_equal "${#lines[@]}" 0
}

# bats tests_tags=search

@test "'pkgdb search' with '_' in search term (negative match)" {
  params="$(genGMParams '.query.match="hello_" | .manifest.options.systems=["x86_64-linux"]')"
  run --separate-stderr "$PKGDB_BIN" search -q --ga-registry "$params"
  assert_success
  assert_equal "${#lines[@]}" 0
}

# bats tests_tags=search

@test "'pkgdb search' with '_' in search term (positive match)" {
  params="$(genGMParams '.query.match="hell_" | .manifest.options.systems=["x86_64-linux"]')"
  run --separate-stderr "$PKGDB_BIN" search -q --ga-registry "$params"
  assert_success
  assert_equal "${#lines[@]}" 4
}

# bats tests_tags=search

@test "'pkgdb search' with '%' in search term" {
  params="$(genGMParams '.query.match="hello%" | .manifest.options.systems=["x86_64-linux"]')"
  run --separate-stderr "$PKGDB_BIN" search -q --ga-registry "$params"
  assert_success
  assert_equal "${#lines[@]}" 0
}

# ---------------------------------------------------------------------------- #

@test "'pkgdb search' works with IFD" {
  run sh -c "NIX_CONFIG=\"allow-import-from-derivation = true\" $PKGDB_BIN search -q --ga-registry --match hello"
  assert_success

  run [ "${#lines[@]}" -gt 0 ]
  assert_success
}

# ---------------------------------------------------------------------------- #

@test "'pkgdb search' doesn't crash when run in parallel" {
  # We don't want other tests polluting parallel test runs so we do this test
  # with a unique cache directory.
  run --separate-stderr sh -c '
    PKGDB_CACHEDIR="$(mktemp -d)" parallel \
      "sleep 1.{}; \"$PKGDB_BIN\" search --ga-registry --match-name hello"\
      ::: $(seq 5)'
  assert_success
  n_lines="${#lines[@]}"
  assert_equal "$n_lines" 40 # 5x number of results from hello
}

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
