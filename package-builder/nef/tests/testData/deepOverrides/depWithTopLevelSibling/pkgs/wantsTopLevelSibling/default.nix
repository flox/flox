# `topLevelValue` coincides with a real top-level name in `basePkgs`
# -- the stub mechanism warns and falls back to that upstream value
# instead of this repository's own `pkgs/topLevelValue.nix`.
{ topLevelValue, ... }: topLevelValue
