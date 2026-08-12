# Pattern: a narrowed forward around an import cycle. The helper is handed
# `myorg` and imports this file back under it, so every lap re-roots the
# forward one component deeper. Judging the forward where it lands rather than
# as written is what ends the walk.
{ catalogs }:
{
  viaHelper = (import ./helper.nix { myorg = catalogs.myorg; }).fromHelper;
}
