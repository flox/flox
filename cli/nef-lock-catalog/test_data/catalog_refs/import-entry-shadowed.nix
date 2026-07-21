# Pattern: an import argument named like the catalog root but bound to
# something else. The import must NOT be followed — the helper's `catalogs`
# parameter is not the catalog namespace here.
{ catalogs, somethingElse }:
let
  helper = import ./import-helper.nix { catalogs = somethingElse; };
in
catalogs.myorg.direct-pkg
