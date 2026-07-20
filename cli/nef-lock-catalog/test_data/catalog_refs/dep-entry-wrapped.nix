# Pattern: the package function is wrapped in `let … in`; the dependency
# argument must still pull the sibling package into the closure.
let
  version = "1.0";
in
{ catalogs, dep-helper }: catalogs.myorg.toolkit.readVersion
