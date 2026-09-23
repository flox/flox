# Requests `topLevelValue` the same way a normal `pkgs/` package would.
# Shares this directory with the decoy below -- the override and an
# ordinary package sit in the same `pkgs/` tree, since a global
# override is itself a publishable package. If this override could
# see repoD's own `pkgs/topLevelValue.nix`, it would receive that
# instead of the shared base's value.
{ topLevelValue }: "saw: ${topLevelValue}"
