# Malformed Nix: the binding is missing its value. rnix recovers and yields a
# partial tree, so an unchecked parse would silently drop `catalogs.myorg.beta`.
{ catalogs }:
{
  alpha = ;
  beta = catalogs.myorg.beta;
}
