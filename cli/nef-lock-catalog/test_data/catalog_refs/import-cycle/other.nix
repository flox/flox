# The other half of the cycle, importing the entry back.
{ catalogs }:
{
  back = import ./entry.nix { inherit catalogs; };
  own = catalogs.myorg.cycle-pkg;
}
