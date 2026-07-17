# Pattern: `catalogs` is forwarded into an import without being declared in
# the file's own arguments. The helper's refs surface under this file's
# `catalogs` namespace, which the expression can never receive.
{ mkDerivation }:
mkDerivation {
  passthru = (import ./helper.nix { inherit catalogs; }).result;
}
