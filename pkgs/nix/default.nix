# ============================================================================ #
#
# Selects the version of `nix' that flox builds against.
#
# The pinned nixpkgs packages 2.35.2 as `nixVersions.latest`. 2.35.3 carries
# the fixes for the advisories published on 2026-10-13 (GHSA-jwvx-xq5j-gc56,
# GHSA-92x5-h528-wrmw, GHSA-m4vq-qf6v-82pj, GHSA-25vp-6h23-hj3g,
# GHSA-hf6w-2rg9-v84g), so re-point the 2.35 component set at that tag with
# `overrideSource`. Setting `src` through `overrideScope` is not enough: with
# an empty patch list the scope keeps the source it already captured and only
# the version string changes.
#
# TODO: reduce this to `nixVersions.latest` once the pinned nixpkgs carries
# 2.35.3. `flox-nix-plugins` must be ported along with every minor bump.
#
# ---------------------------------------------------------------------------- #
{ nixVersions, fetchFromGitHub }:
let
  version = "2.35.3";
  src = fetchFromGitHub {
    owner = "NixOS";
    repo = "nix";
    tag = version;
    hash = "sha256-qtlx8ywd1qj2ToZTNBunXE7JUajLtw44ja3r7+d47D4=";
  };
in
(nixVersions.extend (
  _: prev: {
    nixComponents_2_35 = (prev.nixComponents_2_35.overrideSource src).overrideScope (
      _: _: { inherit version; }
    );
  }
)).nix_2_35
# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
