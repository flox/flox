#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test `flox init' sub-command.
#
# This sub-command's `--template' option takes an "installable" URI as its
# argument, and we focus on auditing various URI edge cases here.
#
# NOTE: the URIs accepted by `flox init --template <URI>' do not accept an
# extended output spec ( `^out,bin,dev' suffix ).
#
#
# ---------------------------------------------------------------------------- #

load test_support.bash;

# bats file_tags=uri, init, uri:project


# ---------------------------------------------------------------------------- #

setup_file() {
  common_file_setup;

  # Suppresses warning messages that clutter backtraces.
  git config --global init.defaultBranch main;

  export _nixpkgs_rev="4ecab3273592f27479a583fb6d975d4aba3486fe";
  export _floxpkgs_rev="2c75b96bc3e8c78b516b1fc44dbf95deae6affca";

  # Ensure we have the `nixpkgs' and `nixpkgs-flox' aliases.
  $FLOX_CLI nix registry add nixpkgs        github:NixOS/nixpkgs;
  $FLOX_CLI nix registry add nixpkgs-flox   github:flox/nixpkgs-flox;
  $FLOX_CLI nix registry add floxpkgs-alias github:flox/floxpkgs;
}


# ---------------------------------------------------------------------------- #

# Helpers for project based tests.

project_setup() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/project-${BATS_TEST_NUMBER?}";
  rm -rf "$PROJECT_DIR";
  mkdir -p "$PROJECT_DIR";
  pushd "$PROJECT_DIR" >/dev/null||return;
  git init;
}

project_teardown() {
  popd >/dev/null||return;
  rm -rf "${PROJECT_DIR?}";
  unset PROJECT_DIR;
}


# ---------------------------------------------------------------------------- #

setup()    { common_test_setup; project_setup;       }
teardown() { project_teardown; common_test_teardown; }


# ---------------------------------------------------------------------------- #

# bats test_tags=uri:github
@test "'flox init -t github:flox/floxpkgs#project'" {
  run "$FLOX_CLI" init -n "${PWD##*/}" -t 'github:flox/floxpkgs#project';
  assert_success;
  # Ensure the template was applied.
  # This is not intended to audit the template's contents, feel free to change
  # this check if the upstream template no longer carries this file.
  assert test -f "./shells/${PWD##*/}/default.nix";
}


# bats test_tags=uri:github
@test "'flox init -t github:flox/floxpkgs/master#project'" {
  run "$FLOX_CLI" init -n "${PWD##*/}" -t 'github:flox/floxpkgs/master#project';
  assert_success;
  assert test -f "./shells/${PWD##*/}/default.nix";
}

# bats test_tags=uri:github
@test "'flox init -t github:flox/floxpkgs?ref=master#project'" {
  run "$FLOX_CLI" init -n "${PWD##*/}"                                \
                       -t 'github:flox/floxpkgs?ref=master#project';
  assert_success;
  assert test -f "./shells/${PWD##*/}/default.nix";
}


# bats test_tags=uri:github
@test "'flox init -t github:flox/floxpkgs/refs/heads/master#project'" {
  run "$FLOX_CLI" init -n "${PWD##*/}"                                       \
                       -t 'github:flox/floxpkgs/refs/heads/master#project';
  assert_success;
  assert test -f "./shells/${PWD##*/}/default.nix";
}


# bats test_tags=uri:github
@test "'flox init -t github:flox/floxpkgs?ref=refs/heads/master#project'" {
  run "$FLOX_CLI" init                                                      \
                  -n "${PWD##*/}"                                           \
                  -t 'github:flox/floxpkgs?ref=refs/heads/master#project';
  assert_success;
  assert test -f "./shells/${PWD##*/}/default.nix";
}


# bats test_tags=uri:github
@test "'flox init -t github:flox/floxpkgs/<REV>#project'" {
  run "$FLOX_CLI" init -n "${PWD##*/}"                                    \
                       -t "github:flox/floxpkgs/$_floxpkgs_rev#project";
  assert_success;
  assert test -f "./shells/${PWD##*/}/default.nix";
}


# bats test_tags=uri:github
@test "'flox init -t github:flox/floxpkgs?rev=<REV>#project'" {
  run "$FLOX_CLI" init -n "${PWD##*/}"                                        \
                       -t "github:flox/floxpkgs?rev=$_floxpkgs_rev#project";
  assert_success;
  assert test -f "./shells/${PWD##*/}/default.nix";
}


# ---------------------------------------------------------------------------- #

# bats test_tags=uri:file, uri:git, uri:git_file
@test "'flox init -t <ABS-PATH>#project'" {
  git clone --depth 1 https://github.com/flox/floxpkgs.git  \
                      "$BATS_TEST_TMPDIR/floxpkgs";
  run "$FLOX_CLI" init -n "${PWD##*/}" -t "$BATS_TEST_TMPDIR/floxpkgs#project";
  assert_success;
  assert test -f "./shells/${PWD##*/}/default.nix";
}

# bats test_tags=uri:file, uri:git, uri:git_file
@test "'flox init -t <REL-PATH>#project'" {
  git clone --depth 1 https://github.com/flox/floxpkgs.git  \
                      "$BATS_TEST_TMPDIR/floxpkgs";
  run "$FLOX_CLI" init -n "${PWD##*/}" -t "../floxpkgs#project";
  assert_success;
  assert test -f "./shells/${PWD##*/}/default.nix";
}


# ---------------------------------------------------------------------------- #

#bats test_tags=uri:indirect, uri:indirect:github
@test "'flox init -t floxpkgs-alias#project'" {
  run "$FLOX_CLI" init -n "${PWD##*/}" -t "floxpkgs-alias#project";
  assert_success;
  assert test -f "./shells/${PWD##*/}/default.nix";
}


#bats test_tags=uri:indirect, uri:indirect:github
@test "'flox init -t flake:floxpkgs-alias#project'" {
  run "$FLOX_CLI" init -n "${PWD##*/}" -t "flake:floxpkgs-alias#project";
  assert_success;
  assert test -f "./shells/${PWD##*/}/default.nix";
}


#bats test_tags=uri:indirect, uri:indirect:github
@test "'flox init -t floxpkgs-alias/master#project'" {
  run "$FLOX_CLI" init -n "${PWD##*/}" -t "floxpkgs-alias/master#project";
  assert_success;
  assert test -f "./shells/${PWD##*/}/default.nix";
}


#bats test_tags=uri:indirect, uri:indirect:github
@test "'flox init -t flake:floxpkgs-alias/master#project'" {
  run "$FLOX_CLI" init -n "${PWD##*/}" -t "flake:floxpkgs-alias/master#project";
  assert_success;
  assert test -f "./shells/${PWD##*/}/default.nix";
}


#bats test_tags=uri:indirect, uri:indirect:github
@test "'flox init -t flake:floxpkgs-alias?ref=master#project'" {
  run "$FLOX_CLI" init -n "${PWD##*/}"                                \
                       -t "flake:floxpkgs-alias?ref=master#project";
  assert_success;
  assert test -f "./shells/${PWD##*/}/default.nix";
}


#bats test_tags=uri:indirect, uri:indirect:github
@test "'flox init -t floxpkgs-alias?ref=master#project' (expect fail)" {
  skip "FIXME: indirect flake-refs require scheme prefix to use parameters.";
  run "$FLOX_CLI" init -n "${PWD##*/}" -t "floxpkgs-alias?ref=master#project";
  assert_failure;
  refute test -f "./shells/${PWD##*/}/default.nix";
}


#bats test_tags=uri:indirect, uri:indirect:github
@test "'flox init -t floxpkgs-alias/refs/heads/master#project' (expect fail)" {
  skip "FIXME: indirect flake-refs require scheme prefix to use parameters.";
  run "$FLOX_CLI" init -n "${PWD##*/}"                                 \
                       -t "floxpkgs-alias/refs/heads/master#project";
  assert_failure;
  refute test -f "./shells/${PWD##*/}/default.nix";
}


#bats test_tags=uri:indirect, uri:indirect:github
@test "'flox init -t flake:floxpkgs-alias?ref=refs/heads/master#project'" {
  run "$FLOX_CLI" init                                                      \
                  -n "${PWD##*/}"                                           \
                  -t "flake:floxpkgs-alias?ref=refs/heads/master#project";
  assert_success;
  assert test -f "./shells/${PWD##*/}/default.nix";
}


# ---------------------------------------------------------------------------- #

# TODO: git+(https|ssh), tarball


# ---------------------------------------------------------------------------- #


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
