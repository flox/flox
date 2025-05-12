{ lib }:
let
  collected = lib.nef.dirToAttrs ./testData/pkgs;
  pkgs = lib.nef.extendAttrSet [ ] { } (import ./testData/basePackageSet.nix {
    inherit lib;
  }) collected;
in

{
  "test: adds package" = {
    expr = pkgs.newPackage;
    expected = "there i am";
  };

  "test: adds package defined as plain files" = {
    expr = pkgs.plainFile;
    expected = "works too";
  };

  "test: overrides existing" = {
    expr = pkgs.topLevelDependency;
    expected = "overridden top-level";
  };

  "test: lazy pacakge throws on eval" = {
    expr = pkgs.lazyEval;
    expectedError = {
      type = "ThrownError";
      msg = "should only throw when accessed";
    };
  };

  "test: remaining attributes sill exist" = {
    expr = pkgs.topLevelValue;
    expected = "value";
  };

  "test: top-level overrides apply to consumers" = {
    expr = pkgs.topLevelDependent;
    expected = "depends on value and overridden top-level";
  };

  "test: top-level overrides apply to consumers in makeExtensible sets" = {
    expr = pkgs.setMakeExtensible.extensibleDependent;
    expected = "depends on value, value and overridden extensible";
  };

  "test: top-level overrides apply to consumers in makeScope sets" = {
    expr = pkgs.setMakeScope.makeScopeDependent;
    expected = "depends on value, overridden top-level and overridden scope";
  };
}
