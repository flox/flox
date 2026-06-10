{ catalogs }:
let
  helper = import ./import-helper.nix { inherit catalogs; };
in {
  result = helper.result;
  extra = catalogs.myorg.extra-pkg;
}
