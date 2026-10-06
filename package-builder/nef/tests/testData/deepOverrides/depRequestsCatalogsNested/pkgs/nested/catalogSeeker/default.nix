# Inside `nested`, so it goes through `callPackageIn`'s fallback branch.
{ catalogs, ... }: builtins.attrNames catalogs
