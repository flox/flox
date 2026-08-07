# Forwards a catalog, whose helper then passes a package-deep path on into a
# further import.
{ catalogs }: (import ./helper.nix { myorg = catalogs.myorg; }).viaDeep
