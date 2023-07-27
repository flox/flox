#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test URI parsers used across various `flox' sub-commands, especially to
# enforce consistency across those sub-commands.
#
# We are concerned with a few types of URIs:
# - flake references, e.g. "github:owner/repo/ref?dir=subdir"
# - installables, a flake reference + attrpath,
#   e.g. "nixpkgs-flox#python3Packages.pip" or
#        ".#legacyPackages.x86_64-linux.foo"
# - flox package reference "unstable.nixpkgs-flox.hello"
# - Environment references, these are the most unstable as they may be either
#   a raw identifier associated with a named environment, or they may be a
#   path + name similar to an installable URI,
#   e.g. "default", "./project#default", "foo", or ".#foo"
#
# These URIs may appear as arguments for CLI commands, as parts of `flox.nix'
# manifests, or as entries in `catalog.json' and `manifest.json'
# ( misleading name ) lock-files.
#
# Relevant commands:
# | init          | env | installable | flake |         |
# | upgrade       | env | installable | flake | floxpkg |
# | subscribe     |     |             | flake |         |
# | unsubscribe   |     |             | flake |         |
# | install       | env | installable | flake | floxpkg |
# | remove        | env | installable | flake | floxpkg |
# | develop       |     | installable | flake |         |
# | build         |     | installable | flake | floxpkg |
# | run           |     | installable | flake | floxpkg |
# | print-dev-env |     | installable | flake | floxpkg |
# | shell         |     | installable | flake | floxpkg |
# | publish       | env | installable | flake | floxpkg |
# | eval          |     | installable | flake |         |
# | search        |     |             | flake |         |
#
#
# ---------------------------------------------------------------------------- #

load test_support.bash;

# bats file_tags=uri, uri:project


# ---------------------------------------------------------------------------- #

setup_file() {
  common_file_setup;

  # Suppresses warning messages that clutter backtraces.
  git config --global init.defaultBranch main;

  export _nixpkgs_rev="4ecab3273592f27479a583fb6d975d4aba3486fe";
  export _floxpkgs_rev="2c75b96bc3e8c78b516b1fc44dbf95deae6affca";

  # Ensure we have the `nixpkgs' and `nixpkgs-flox' aliases.
  $FLOX_CLI nix registry add nixpkgs      github:NixOS/nixpkgs;
  $FLOX_CLI nix registry add nixpkgs-flox github:flox/nixpkgs-flox;

  #declare -a envRefs flakeRefs installables floxpkgRefs;
  #envRefs=(
  #  default  # either project or global default named env
  #  foo      # either project or global named env
  #  .        # project default named env
  #  .#foo    # project named env
  #  ./foo    # project subdir env
  #);

  ## Define a few flake refs
  #flakeRefs=(
  #  # indirect flake
  #  nixpkgs                         # with implied scheme
  #  flake:nixpkgs                   # with explicit scheme
  #  nixpkgs/23.05                   # with path ref
  #  flake:nixpkgs/23.05
  #  'nixpkgs?ref=23.05'             # with query ref
  #  ## XXX this fails with `nix' v2.15.x:  "nixpkgs?ref=refs/heads/23.05"
  #  'flake:nixpkgs?ref=refs/heads/23.05'  # full ref
  #  "nixpkgs/$_nixpkgs_rev"         # with path rev
  #  "flake:nixpkgs/$_nixpkgs_rev"
  #  "flake:nixpkgs?rev=$_nixpkgs_rev"

  #  # indirect flake in subdir ( no packages )
  #  'nixpkgs?dir=lib'
  #  'flake:nixpkgs?dir=lib'
  #  'nixpkgs/23.05?dir=lib'
  #  'flake:nixpkgs/23.05?dir=lib'
  #  'flake:nixpkgs?ref=23.05&dir=lib'
  #  'flake:nixpkgs?ref=refs/heads/23.05&dir=lib'
  #  "nixpkgs/$_nixpkgs_rev?dir=lib"
  #  "flake:nixpkgs/$_nixpkgs_rev?dir=lib"
  #  "flake:nixpkgs?rev=$_nixpkgs_rev&dir=lib"

  #  # github flake
  #  github:NixOS/nixpkgs
  #  github:NixOS/nixpkgs/23.05
  #  "github:NixOS/nixpkgs/$_nixpkgs_rev"
  #  'github:NixOS/nixpkgs?dir=lib'
  #  'github:NixOS/nixpkgs/23.05?dir=lib'
  #  'github:NixOS/nixpkgs?ref=23.05&dir=lib'
  #  "github:NixOS/nixpkgs/$_nixpkgs_rev?dir=lib"
  #  "github:NixOS/nixpkgs?rev=$_nixpkgs_rev&dir=lib"

  #  # git flake
  #  'git:git@github.com/NixOS/nixpkgs'
  #  'git:git@github.com/NixOS/nixpkgs?ref=23.05&allRefs=1'
  #  "git:git@github.com/NixOS/nixpkgs?rev=$_nixpkgs_rev"
  #  'git:git@github.com/NixOS/nixpkgs?dir=lib'
  #  "git:git@github.com/NixOS/nixpkgs?ref=23.05&dir=lib"
  #  "git:git@github.com/NixOS/nixpkgs?rev=$_nixpkgs_rev&dir=lib"
  #  'git+ssh://git@github.com/NixOS/nixpkgs'
  #  'git+ssh://git@github.com/NixOS/nixpkgs?ref=23.05&allRefs=1'
  #  "git+ssh://git@github.com/NixOS/nixpkgs?rev=$_nixpkgs_rev"
  #  'git+ssh://git@github.com/NixOS/nixpkgs?dir=lib'
  #  "git+ssh://git@github.com/NixOS/nixpkgs?ref=23.05&dir=lib"
  #  "git+ssh://git@github.com/NixOS/nixpkgs?rev=$_nixpkgs_rev&dir=lib"
  #  'git+https://git@github.com/NixOS/nixpkgs'
  #  'git+https://git@github.com/NixOS/nixpkgs?ref=23.05&allRefs=1'
  #  "git+https://git@github.com/NixOS/nixpkgs?rev=$_nixpkgs_rev"
  #  'git+https://git@github.com/NixOS/nixpkgs?dir=lib'
  #  "git+https://git@github.com/NixOS/nixpkgs?ref=23.05&dir=lib"
  #  "git+https://git@github.com/NixOS/nixpkgs?rev=$_nixpkgs_rev&dir=lib"

  #  # indirect catalog
  #  nixpkgs-flox
  #  flake:nixpkgs-flox
  #);
  #installables=();
  #floxpkgRefs=();

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

# `flox init' tests
# -----------------

# bats file_tags=uri, uri:project, init

@test "'flox init -t github:flox/floxpkgs#project'" {
  run "$FLOX_CLI" init -n "${PWD##*/}" -t 'github:flox/floxpkgs#project';
  assert_success;
  # Ensure the template was applied.
  # This is not intended to audit the template's contents, feel free to change
  # this check if the upstream template no longer carries this file.
  assert test -f "./shells/${PWD##*/}/default.nix";
}


@test "'flox init -t github:flox/floxpkgs/master#project'" {
  run "$FLOX_CLI" init -n "${PWD##*/}" -t 'github:flox/floxpkgs/master#project';
  assert_success;
  assert test -f "./shells/${PWD##*/}/default.nix";
}

@test "'flox init -t github:flox/floxpkgs?ref=master#project'" {
  run "$FLOX_CLI" init -n "${PWD##*/}"                                \
                       -t 'github:flox/floxpkgs?ref=master#project';
  assert_success;
  assert test -f "./shells/${PWD##*/}/default.nix";
}


@test "'flox init -t github:flox/floxpkgs/refs/heads/master#project'" {
  run "$FLOX_CLI" init -n "${PWD##*/}"                                       \
                       -t 'github:flox/floxpkgs/refs/heads/master#project';
  assert_success;
  assert test -f "./shells/${PWD##*/}/default.nix";
}


@test "'flox init -t github:flox/floxpkgs?ref=refs/heads/master#project'" {
  run "$FLOX_CLI" init                                                      \
                  -n "${PWD##*/}"                                           \
                  -t 'github:flox/floxpkgs?ref=refs/heads/master#project';
  assert_success;
  assert test -f "./shells/${PWD##*/}/default.nix";
}


@test "'flox init -t github:flox/floxpkgs/<REV>#project'" {
  run "$FLOX_CLI" init -n "${PWD##*/}"                                    \
                       -t "github:flox/floxpkgs/$_floxpkgs_rev#project";
  assert_success;
  assert test -f "./shells/${PWD##*/}/default.nix";
}


@test "'flox init -t github:flox/floxpkgs?rev=<REV>#project'" {
  run "$FLOX_CLI" init -n "${PWD##*/}"                                        \
                       -t "github:flox/floxpkgs?rev=$_floxpkgs_rev#project";
  assert_success;
  assert test -f "./shells/${PWD##*/}/default.nix";
}


@test "'flox init -t <ABS-PATH>#project'" {
  git clone --depth 1 https://github.com/flox/floxpkgs.git  \
                      "$BATS_TEST_TMPDIR/floxpkgs";
  run "$FLOX_CLI" init -n "${PWD##*/}" -t "$BATS_TEST_TMPDIR/floxpkgs#project";
  assert_success;
  assert test -f "./shells/${PWD##*/}/default.nix";
}


@test "'flox init -t <REL-PATH>#project'" {
  git clone --depth 1 https://github.com/flox/floxpkgs.git  \
                      "$BATS_TEST_TMPDIR/floxpkgs";
  run "$FLOX_CLI" init -n "${PWD##*/}" -t "../floxpkgs#project";
  assert_success;
  assert test -f "./shells/${PWD##*/}/default.nix";
}

# TODO: git, tarball


# ---------------------------------------------------------------------------- #

# bats file_tags=uri, uri:project


# TODO: develop
# TODO: build
# TODO: run
# TODO: print-dev-env
# TODO: shell
# TODO: eval


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
