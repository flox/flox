{ catalogs }:
let
  helper = import ./import-helper.nix { catalogs = catalogs; };
in helper.result
