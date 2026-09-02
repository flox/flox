# ============================================================================ #
#
# Selects the `nix' used by flox, and applies patches to it.
#
# N.B. we select the "stable" revision in the full knowledge/expectation that
# this will change out from underneath us over time. We do this along with a
# commitment to continually build against the (flox) nixpkgs staging and
# unstable branches in CI so that we will be notified when upcoming upgrades of
# nixVersions.stable upstream break our build.
#
# ---------------------------------------------------------------------------- #
{ nixVersions }:
nixVersions.stable
# .appendPatches [
#   # E.g:
#   # (builtins.path { path = ./patches/pr_<PR>_<description>.patch; })
# ]
# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
