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
# | create        | env |             |       |         |
# | init          | env | installable | flake |         |
# | activate      | env |             |       |         |
# | list          | env |             |       |         |
# | edit          | env |             |       |         |
# | destroy       | env |             |       |         |
# | rollback      | env |             |       |         |
# | upgrade       | env | installable | flake | floxpkg |
# | push          | env |             |       |         |
# | pull          | env |             |       |         |
# | bundle        | env |             |       |         |
# | containerize  | env |             |       |         |
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

# bats file_tags=uri


# ---------------------------------------------------------------------------- #

setup_file() {
  common_file_setup;

  declare -a envRefs flakeRefs installables floxpkgRefs;
  envRefs=(
    default  # either project or global default named env
    foo      # either project or global named env
    .        # project default named env
    .#foo    # project named env
    ./foo    # project subdir env
  );
  export _nixpkgs_rev="4ecab3273592f27479a583fb6d975d4aba3486fe";

  # Ensure we have the `nixpkgs' and `nixpkgs-flox' aliases.
  $FLOX_CLI nix registry add nixpkgs      github:NixOS/nixpkgs;
  $FLOX_CLI nix registry add nixpkgs-flox github:flox/nixpkgs-flox;

  # Define a few flake refs
  flakeRefs=(
    # indirect flake
    nixpkgs                         # with implied scheme
    flake:nixpkgs                   # with explicit scheme
    nixpkgs/23.05                   # with path ref
    flake:nixpkgs/23.05
    'nixpkgs?ref=23.05'             # with query ref
    ## XXX this fails with `nix' v2.15.x:  "nixpkgs?ref=refs/heads/23.05"
    'flake:nixpkgs?ref=refs/heads/23.05'  # full ref
    "nixpkgs/$_nixpkgs_rev"         # with path rev
    "flake:nixpkgs/$_nixpkgs_rev"
    "flake:nixpkgs?rev=$_nixpkgs_rev"

    # indirect flake in subdir ( no packages )
    'nixpkgs?dir=lib'
    'flake:nixpkgs?dir=lib'
    'nixpkgs/23.05?dir=lib'
    'flake:nixpkgs/23.05?dir=lib'
    'flake:nixpkgs?ref=23.05&dir=lib'
    'flake:nixpkgs?ref=refs/heads/23.05&dir=lib'
    "nixpkgs/$_nixpkgs_rev?dir=lib"
    "flake:nixpkgs/$_nixpkgs_rev?dir=lib"
    "flake:nixpkgs?rev=$_nixpkgs_rev&dir=lib"

    # github flake
    github:NixOS/nixpkgs
    github:NixOS/nixpkgs/23.05
    "github:NixOS/nixpkgs/$_nixpkgs_rev"
    'github:NixOS/nixpkgs?dir=lib'
    'github:NixOS/nixpkgs/23.05?dir=lib'
    'github:NixOS/nixpkgs?ref=23.05&dir=lib'
    "github:NixOS/nixpkgs/$_nixpkgs_rev?dir=lib"
    "github:NixOS/nixpkgs?rev=$_nixpkgs_rev&dir=lib"

    # git flake
    'git:git@github.com/NixOS/nixpkgs'
    'git:git@github.com/NixOS/nixpkgs?ref=23.05&allRefs=1'
    "git:git@github.com/NixOS/nixpkgs?rev=$_nixpkgs_rev"
    'git:git@github.com/NixOS/nixpkgs?dir=lib'
    "git:git@github.com/NixOS/nixpkgs?ref=23.05&dir=lib"
    "git:git@github.com/NixOS/nixpkgs?rev=$_nixpkgs_rev&dir=lib"
    'git+ssh://git@github.com/NixOS/nixpkgs'
    'git+ssh://git@github.com/NixOS/nixpkgs?ref=23.05&allRefs=1'
    "git+ssh://git@github.com/NixOS/nixpkgs?rev=$_nixpkgs_rev"
    'git+ssh://git@github.com/NixOS/nixpkgs?dir=lib'
    "git+ssh://git@github.com/NixOS/nixpkgs?ref=23.05&dir=lib"
    "git+ssh://git@github.com/NixOS/nixpkgs?rev=$_nixpkgs_rev&dir=lib"
    'git+https://git@github.com/NixOS/nixpkgs'
    'git+https://git@github.com/NixOS/nixpkgs?ref=23.05&allRefs=1'
    "git+https://git@github.com/NixOS/nixpkgs?rev=$_nixpkgs_rev"
    'git+https://git@github.com/NixOS/nixpkgs?dir=lib'
    "git+https://git@github.com/NixOS/nixpkgs?ref=23.05&dir=lib"
    "git+https://git@github.com/NixOS/nixpkgs?rev=$_nixpkgs_rev&dir=lib"

    # indirect catalog
    nixpkgs-flox
    flake:nixpkgs-flox
  );
  installables=();
  floxpkgRefs=();

}



# ---------------------------------------------------------------------------- #

@test "'flox activate' " {
  run sh -c "$_inline_cmd";
  assert_success;
  assert_output --partial - < tests/hello-cowsay.out;
}


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
