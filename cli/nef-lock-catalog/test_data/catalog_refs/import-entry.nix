{ catalogs }:
let
  helper = import ./import-helper.nix { inherit catalogs; };
in
helper.result
