{
  lib,
  nixpkgs,
  fixtures,
}:
let
  instantiate = lib.nef.instantiate;

  singleLevel = instantiate {
    inherit nixpkgs;
    sourceInfo = builtins.fetchTree "path:${fixtures}/single-level/root";
  };

  multiLevel = instantiate {
    inherit nixpkgs;
    sourceInfo = builtins.fetchTree "path:${fixtures}/multi-level/root";
  };
in
{
  # Single-level catalog tests

  "test: single-level catalog is resolved" = {
    expr = singleLevel.catalogs ? child;
    expected = true;
  };

  "test: single-level catalog package is accessible" = {
    expr = singleLevel.pkgs.hello;
    expected = "i am dep";
  };

  "test: packages in catalogs can access packages local to them" = {
    expr = singleLevel.pkgs.helloWithUseOfAmbient;
    expected = ''dep says "i am dep"'';
  };

  "test: catalog package is not exposed in root namespace" = {
    expr = singleLevel.pkgs ? dep;
    expected = false;
  };

  # Multi-level catalog tests

  "test: multi-level root catalog is resolved" = {
    expr = multiLevel.catalogs ? mid;
    expected = true;
  };

  "test: sub catalog can be accessed from raw nef output" = {
    expr = multiLevel.catalogInstances.mid.catalogs ? leaf;
    expected = true;
  };

  "test: multi-level package resolves through chain" = {
    expr = multiLevel.pkgs.hello;
    expected = "i am leaf";
  };
}
