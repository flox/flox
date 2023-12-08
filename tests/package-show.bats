#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test of rust impl of 'flox show'
#
# bats file_tags=search,show
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# ---------------------------------------------------------------------------- #

# Helpers for project based tests.

project_setup() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/test"
  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" >/dev/null || return
  run "$FLOX_CLI" init;
  assert_success;
  unset output;
}

project_teardown() {
  popd >/dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
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

# ---------------------------------------------------------------------------- #

@test "'flox show' can be called at all" {
  run "$FLOX_CLI" show hello;
  assert_success;
}


# ---------------------------------------------------------------------------- #

@test "'flox show' accepts specific input" {
  skip DEPRECATED;
  run "$FLOX_CLI" show nixpkgs-flox:hello;
  assert_success;
  # TODO: better testing once the formatting is implemented
}

# ---------------------------------------------------------------------------- #

@test "'flox show' accepts search output without separator" {
  run "$FLOX_CLI" search hello;
  assert_success;
  first_result="${lines[0]%% *}";
  run "$FLOX_CLI" show "$first_result";
  assert_success;
}


# ---------------------------------------------------------------------------- #

@test "'flox show' accepts search output with separator" {
  skip DEPRECATED;
  run "$FLOX_CLI" search nixpkgs-flox:hello;
  assert_success;
  first_result="${lines[0]%% *}";
  run "$FLOX_CLI" show "$first_result";
  assert_success;
}


# ---------------------------------------------------------------------------- #

@test "'flox show' - hello" {
  run "$FLOX_CLI" show hello;
  assert_success;
  assert_equal "${lines[0]}" "hello - A program that produces a familiar, friendly greeting";
  assert_equal "${lines[1]}" "    hello - hello@2.12.1";
}


# ---------------------------------------------------------------------------- #

@test "'flox show' - hello --all" {
  run "$FLOX_CLI" show hello --all;
  assert_success;
  assert_equal "${lines[0]}" "hello - A program that produces a familiar, friendly greeting";
  assert_equal "${lines[1]}" "    hello - hello@2.12.1";
}


# ---------------------------------------------------------------------------- #

@test "'flox show' - python27Full" {
  run "$FLOX_CLI" show python27Full;
  assert_success;
  assert_equal "${lines[0]}" "python27Full - A high-level dynamically-typed programming language";
  assert_equal "${lines[1]}" "    python27Full - python27Full@2.7.18.6";
}


# ---------------------------------------------------------------------------- #

@test "'flox show' - python27Full --all" {
  run "$FLOX_CLI" show python27Full --all;
  assert_success;
  assert_equal "${lines[0]}" "python27Full - A high-level dynamically-typed programming language";
  assert_equal "${lines[1]}" "    python27Full - python27Full@2.7.18.6";
}


# ---------------------------------------------------------------------------- #

@test "'flox show' works in project without manifest or lockfile" {
  rm -f "$PROJECT_DIR/.flox/manifest.toml";
  run --separate-stderr "$FLOX_CLI" show hello;
  assert_success;
}


# ---------------------------------------------------------------------------- #

@test "'flox show' works outside of projects" {
  rm -rf "$PROJECT_DIR/.flox";
  run --separate-stderr "$FLOX_CLI" show hello;
  assert_success;
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
    }'|head -n1|jq -r '.version';";
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

@test "'flox show' prompts when an environment is activated and there is an environment in the current directory" {
  # Set up two environments locked to different revisions of nixpkgs, and
  # confirm that flox show displays different versions of nodejs for each.
  
  mkdir 1
  pushd 1
  "$FLOX_CLI" init
  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_OLD?}" \
    "$FLOX_CLI" --debug install nodejs

  run --separate-stderr sh -c "$FLOX_CLI show nodejs|tail -n1";
  assert_success;
  assert_output '    nodejs - nodejs@18.16.0';
  popd
  

  mkdir 2
  pushd 2
  "$FLOX_CLI" init
  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_NEW?}" \
    "$FLOX_CLI" install nodejs
  
  run --separate-stderr sh -c "$FLOX_CLI show nodejs|tail -n1";
  assert_success;
  assert_output '    nodejs - nodejs@18.17.1';
  popd

  SHELL=bash run expect -d "$TESTS_DIR/show/prompt-which-environment.exp"
  assert_success
}
