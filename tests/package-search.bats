#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test of rust impl of `flox search`
#
# bats file_tags=search
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# ---------------------------------------------------------------------------- #

# Helpers for project based tests.

project_setup() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/test";
  rm -rf "$PROJECT_DIR";
  mkdir -p "$PROJECT_DIR";
  pushd "$PROJECT_DIR" >/dev/null||return;
  run "$FLOX_CLI" init;
  assert_success;
  unset output;
}

project_teardown() {
  popd >/dev/null||return;
  rm -rf "${PROJECT_DIR?}";
  unset PROJECT_DIR;
}


# ---------------------------------------------------------------------------- #

setup() {
  common_test_setup;
  project_setup;
}

teardown() {
  project_teardown;
  common_test_teardown;
}

setup_file() {
  common_file_setup;

  export SHOW_HINT="Use \`flox show {package}\` to see available versions"
  # Separator character for ambiguous package sources
  export SEP=":";

  if [[ -z "${PKGDB_BIN}" ]]; then
    echo "You must set \$PKGDB_BIN to run these tests" >&2;
    exit 1;
  fi
}


# ---------------------------------------------------------------------------- #

@test "'flox search' can be called at all" {
  run "$FLOX_CLI" search hello;
  assert_success;
}


# ---------------------------------------------------------------------------- #

@test "'flox search' error with no search term" {
  run "$FLOX_CLI" search;
  assert_failure;
}


# ---------------------------------------------------------------------------- #

@test "'flox search' helpful error with unquoted redirect: hello@>1 -> hello@" {
  run "$FLOX_CLI" search hello@;
  assert_failure;
  assert_output --partial "try quoting";
}


# ---------------------------------------------------------------------------- #

@test "'FLOX_FEATURES_SEARCH_STRATEGY=match flox search' expected number of results: 'hello'" {
  FLOX_FEATURES_SEARCH_STRATEGY=match run --separate-stderr "$FLOX_CLI" search hello;
  n_lines="${#lines[@]}";
  case "$NIX_SYSTEM" in
    *-darwin)
      assert_equal "$n_lines" 11;
      assert_equal "$stderr" "$SHOW_HINT"
      ;;
    *-linux)
      assert_equal "$n_lines" 11;
      assert_equal "$stderr" "$SHOW_HINT"
      ;;
  esac
}


# ---------------------------------------------------------------------------- #

@test "'flox search' expected number of results: 'hello'" {
  run --separate-stderr "$FLOX_CLI" search hello;
  n_lines="${#lines[@]}";
  case "$NIX_SYSTEM" in
    *-darwin)
      assert_equal "$n_lines" 10;
      assert_equal "$stderr" "$SHOW_HINT"
      ;;
    *-linux)
      assert_equal "$n_lines" 10;
      assert_equal "$stderr" "$SHOW_HINT"
      ;;
  esac
}


# ---------------------------------------------------------------------------- #

@test "'flox search' semver search: hello@2.12.1" {
  run "$FLOX_CLI" search hello@2.12.1;
  n_lines="${#lines[@]}";
  assert_equal "$n_lines" 2; # search line + show hint
  assert_equal "${lines[-1]}" "$SHOW_HINT"
}


# ---------------------------------------------------------------------------- #

@test "'flox search' returns JSON" {
  run "$FLOX_CLI" search hello --json;
  version="$(echo "$output" | jq '.[0].version')";
  assert_equal "$version" '"2.12.1"';
}


# ---------------------------------------------------------------------------- #

@test "'flox search' semver search: 'hello@>=1'" {
  run "$FLOX_CLI" search 'hello@>=1' --json;
  versions="$(echo "$output" | jq -c 'map(.version)')";
  case $THIS_SYSTEM in
    *-darwin)
      assert_equal "$versions" '["2.12.1","2.12","2.10"]';
      ;;
    *-linux)
      assert_equal "$versions" '[]';
      ;;
  esac
}


# ---------------------------------------------------------------------------- #

@test "'flox search' semver search: hello@2.x" {
  run "$FLOX_CLI" search hello@2.x --json;
  versions="$(echo "$output" | jq -c 'map(.version)')";
  assert_equal "$versions" '["2.12.1"]';
}


# ---------------------------------------------------------------------------- #

@test "'flox search' semver search: hello@=2.10" {
  run "$FLOX_CLI" search hello@=2.12;
  n_lines="${#lines[@]}";
  assert_equal "$n_lines" "2"; # search line + show hint
  assert_equal "${lines[-1]}" "$SHOW_HINT"
}


# ---------------------------------------------------------------------------- #

@test "'flox search' semver search: hello@v2" {
  run "$FLOX_CLI" search hello@v2 --json;
  versions="$(echo "$output" | jq -c 'map(.version)')";
  assert_equal "$versions" '["2.12.1"]';
}


# ---------------------------------------------------------------------------- #

@test "'flox search' semver search: 'hello@>1 <3'" {
  run "$FLOX_CLI" search 'hello@>1 <3' --json;
  versions="$(echo "$output" | jq -c 'map(.version)')";
  assert_equal "$versions" '["2.12.1"]';
}


# ---------------------------------------------------------------------------- #

@test "'flox search' exact semver match listed first" {
  run "$FLOX_CLI" search hello@2.12.1 --json;
  first_line="$(echo "$output" | head -n 1 | grep 2.12.1)";
  assert [ -n first_line ];
}


# ---------------------------------------------------------------------------- #

@test "'flox search' displays ambiguous packages with separator" {

  skip "DEPRECATED"

  run "$FLOX_CLI" search hello;
  assert_output --partial "nixpkgs2${SEP}";
  assert_output --partial "nixpkgs-flox${SEP}"
  run "$FLOX_CLI" unsubscribe nixpkgs2;
  assert_success;
}


# ---------------------------------------------------------------------------- #

@test "'flox search' displays unambiguous packages without separator" {
  run "$FLOX_CLI" search hello;
  packages="$(echo "$output" | cut -d ' ' -f 1)";
  case $THIS_SYSTEM in
    *-darwin)
      assert_equal "$packages" "hello";
      ;;
    *-linux)
      # $'foo' syntax allows you to put backslash escapes in literal strings
      assert_equal "$packages" $'hello\nhello-wayland\ngnome.iagno';
      ;;
  esac
}

# ---------------------------------------------------------------------------- #

@test "'flox search' hints at 'flox show'" {
  run --separate-stderr "$FLOX_CLI" search hello;
  assert_success
  assert_equal "$stderr" "$SHOW_HINT"
}


# ---------------------------------------------------------------------------- #

@test "'flox search' error message when no results" {
  run "$FLOX_CLI" search surely_doesnt_exist;
  assert_equal "${#lines[@]}" 1;
  # There's a leading `ERROR: ` that's left off when run non-interactively
  assert_output "No packages matched this search term: surely_doesnt_exist";
}

# ---------------------------------------------------------------------------- #

@test "'flox search' with 'FLOX_FEATURES_SEARCH_STRATEGY=match-name' shows fewer packages" {

  MATCH="$(FLOX_FEATURES_SEARCH_STRATEGY=match "$FLOX_CLI" search node | wc -l)";
  MATCH_NAME="$(FLOX_FEATURES_SEARCH_STRATEGY=match-name "$FLOX_CLI" search node | wc -l)";

  assert [ "$MATCH_NAME" -lt "$MATCH" ];

}

# ---------------------------------------------------------------------------- #

@test "'flox search' works in project without manifest or lockfile" {
  rm -f "$PROJECT_DIR/.flox/manifest.toml";
  run --separate-stderr "$FLOX_CLI" search hello;
  assert_success;
  n_lines="${#lines[@]}";
  assert_equal "$n_lines" 10; # search results from global manifest registry
}


# ---------------------------------------------------------------------------- #

@test "'flox search' works outside of projects" {
  rm -rf "$PROJECT_DIR/.flox";
  run --separate-stderr "$FLOX_CLI" search hello;
  assert_success;
  n_lines="${#lines[@]}";
  assert_equal "$n_lines" 10; # search results from global manifest registry
}


# ---------------------------------------------------------------------------- #

# bats test_tags=search:project, search:manifest, search:show

@test "'flox show' uses '_PKGDB_GA_REGISTRY_REF_OR_REV' revision" {
  mkdir -p "$PROJECT_DIR/.flox/env";
  # Note: at some point it may also be necessary to create a .flox/env.json
  echo 'options.systems = ["x86_64-linux"]'       \
       > "$PROJECT_DIR/.flox/env/manifest.toml";

  # Search for a package with `pkgdb`
  run --separate-stderr sh -c "$PKGDB_BIN search --ga-registry '{
      \"manifest\": \"$PROJECT_DIR/.flox/env/manifest.toml\",
      \"query\": { \"match-name\": \"nodejs\" }
    }'|head -n1|jq -r '.version';"
  assert_success;
  assert_output '18.16.0';
  unset output;

  # Ensure the version of `nodejs' in our search results aligns with the
  # `--ga-registry` default ( 18.16.0 ).
  run --separate-stderr sh -c "$FLOX_CLI show nodejs|tail -n1";
  assert_success;
  assert_output '    nodejs - nodejs@18.16.0';
}


# ---------------------------------------------------------------------------- #

# bats test_tags=search:project, search:manifest, search:lockfile, search:show

@test "'flox show' uses locked revision when available" {
  mkdir -p "$PROJECT_DIR/.flox/env";
  # Note: at some point it may also be necessary to create a .flox/env.json
  {
    echo 'options.systems = ["x86_64-linux"]';
    echo 'install.nodejs = {}';
  } > "$PROJECT_DIR/.flox/env/manifest.toml";

  # Force lockfile to pin a specific revision of `nixpkgs'
  run --separate-stderr sh -c                                          \
   "_PKGDB_GA_REGISTRY_REF_OR_REV='${PKGDB_NIXPKGS_REV_NEW?}'          \
      $PKGDB_BIN manifest lock                                         \
                 --ga-registry '$PROJECT_DIR/.flox/env/manifest.toml'  \
                 > '$PROJECT_DIR/.flox/env/manifest.lock';";
  assert_success;
  unset output;

  # Ensure the locked revision is what we expect.
  run --separate-stderr jq -r '.registry.inputs.nixpkgs.from.rev'      \
                              "$PROJECT_DIR/.flox/env/manifest.lock";
  assert_success;
  assert_output "$PKGDB_NIXPKGS_REV_NEW";
  unset output;

  # Search for a package with `pkgdb`
  run --separate-stderr sh -c                                    \
   "_PKGDB_GA_REGISTRY_REF_OR_REV='$PKGDB_NIXPKGS_REV_NEW'       \
      $PKGDB_BIN search --ga-registry '{
        \"manifest\": \"$PROJECT_DIR/.flox/env/manifest.toml\",
        \"lockfile\": \"$PROJECT_DIR/.flox/env/manifest.lock\",
        \"query\": { \"match-name\": \"nodejs\" }
      }'|head -n1|jq -r '.version';"
  assert_success;
  assert_output '18.17.1';
  unset output;

  # Ensure the version of `nodejs' in our search results aligns with the
  # locked rev ( 18.17.1 ), instead of the `--ga-registry` default ( 18.16.0 ).
  run --separate-stderr sh -c "$FLOX_CLI show nodejs|tail -n1";
  assert_success;
  assert_output '    nodejs - nodejs@18.17.1';
}


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
