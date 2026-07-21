# Pattern: the import function is bound to a name before being applied.
# `mk { inherit catalogs; }` must follow the import like a direct
# `import ./import-helper.nix { inherit catalogs; }`.
{ catalogs }:
let
  mk = import ./import-helper.nix;
in
(mk { inherit catalogs; }).result
