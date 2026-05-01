# ============================================================================ #
#
# Applies patches to `nix' and fixes up public header `#includes'.
#
# Additionally there's a wonky spot where they
# `#include "nlohmann/json_fwd.hpp"' in `include/nix/json-impls.hh' which forces
# consumers to use `-I' instead of `-isystem' for `nlohmann_json' when compiling
# against `nix'.
# This fixes that issue too.
#
# N.B. we select the "stable" revision in the full knowledge/expectation that
# this will change out from underneath us over time. We do this along with a
# commitment to continually build against the (flox) nixpkgs staging and
# unstable branches in CI so that we will be notified when upcoming upgrades of
# nixVersions.stable upstream break our build.
#
# ---------------------------------------------------------------------------- #
{
  nixVersions,
  fetchFromGitHub,
}:
let
  nixVersion = "stable";
in
# TODO: revert to nixpkgs' nix with next nixpkgs bump in May '26.
#
# All versions of nix on the current flox/nixpkgs/stable (2026-03-24)
# are susceptible to GHSA-g3g9-5vj6-r3gj <https://discourse.nixos.org/t/nix-security-advisory-privilege-escalation-via-symlink-following-during-fod-output-registration/76900>.inherit
# We bump the "stable" nix (which refers to `2.31.3`), to 2.31.4
# which includes fixes against the above vuln.
# FWIW, using `.appendPatches` apparently runs into build issues in CI,
# likely on account of <https://github.com/NixOS/nix/issues/14751>.
(nixVersions.extend (
  final: prev: {
    nixComponents_2_31 = prev.nixComponents_2_31.override {
      version = "2.31.4";
      src = fetchFromGitHub {
        owner = "NixOS";
        repo = "nix";
        tag = "2.31.4";
        hash = "sha256-f/haYfcI+9IiYVH+g6cjhF8cK7QWHAFfcPtF+57ujZ0=";
      };
    };

  }
)).

"${nixVersion}"
#  .appendPatches
#   # E.g:
#   # (builtins.path { path = ./patches/pr_<PR>_<description>.patch; })

#   # <https://discourse.nixos.org/t/nix-security-advisory-privilege-escalation-via-symlink-following-during-fod-output-registration/76900>
#   # TODO: drop with next stability bump in May '26 as that will ship with the fix applied.
#   (builtins.path { path = ./patches/GHSA-g3g9-5vj6-r3gj-2.31.3.patch; })
# ]
# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
