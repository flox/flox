# Pattern: two files import each other; the drain must terminate and still
# collect each file's refs.
{ catalogs }:
{
  entry = catalogs.myorg.entry-pkg;
  other = import ./other.nix { inherit catalogs; };
}
