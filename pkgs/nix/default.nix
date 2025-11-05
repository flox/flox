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
# unstable branches in CI so that we will notified when upcoming upgrades of
# nixVersions.stable upstream break our build.
#
# ---------------------------------------------------------------------------- #
{
  nixVersions,
  stdenv,
}:
let
  nixVersion = "stable";
in
nixVersions."${nixVersion}".overrideAttrs (prev: {
  # Necessary for compiling with debug symbols
  inherit stdenv;

  # Apply patch files.
  patches = (prev.patches or [ ]) ++ [
    # E.g:
    # (builtins.path { path = ./patches/pr_<PR>_<description>.patch; })
  ];
})
# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
