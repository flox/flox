#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# `flox-nixpkgs' wrapped input tests.
#
# ---------------------------------------------------------------------------- #

load setup_suite.bash

# bats file_tags=flox-nixpkgs

# ---------------------------------------------------------------------------- #

@test "'github' fetcher does NOT set 'allowUnfree' and 'allowBroken'" {
  run --separate-stderr nix --option plugin-files "$NIX_PLUGINS" \
    eval --expr "let
    nixpkgs = builtins.getFlake \"github:NixOS/nixpkgs/$TEST_NIXPKGS_REV_NEW\";
    inherit (nixpkgs.legacyPackages.x86_64-linux) config;
  in assert ! ( config.allowUnfree || config.allowBroken ); true"
  assert_success
  assert_output "true"
}

# ---------------------------------------------------------------------------- #

@test "'flox-nixpkgs' fetcher sets 'allowUnfree' and 'allowBroken'" {
  run --separate-stderr nix --option plugin-files "$NIX_PLUGINS" \
    eval --expr "let
    nixpkgs = builtins.getFlake
                \"flox-nixpkgs:v0/flox/$TEST_NIXPKGS_REV_NEW\";
    inherit (nixpkgs.legacyPackages.x86_64-linux) config;
  in assert config.allowUnfree && config.allowBroken; true"
  assert_success
  assert_output "true"
}

# ---------------------------------------------------------------------------- #

@test "'flox-nixpkgs' and 'github' 'outPaths' match" {
  run --separate-stderr nix --option plugin-files "$NIX_PLUGINS" \
    eval --expr "let
    fp0 = builtins.getFlake
            \"flox-nixpkgs\:v0/flox/$TEST_NIXPKGS_REV_NEW\";
    op0 = fp0.legacyPackages.x86_64-linux.hello.outPath;

    fp1 = builtins.getFlake \"github:NixOS/nixpkgs/$TEST_NIXPKGS_REV_NEW\";
    op1 = fp1.legacyPackages.x86_64-linux.hello.outPath;

  in assert op0 == op1; true"
  assert_success
  assert_output "true"
}

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
