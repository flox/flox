{
  nixpkgs-url ? "nixpkgs",
  nixpkgs-flake ? builtins.getFlake nixpkgs-url,
}:
let
  nixpkgsFlake = builtins.getFlake "nixpkgs";
  nixpkgs = import nixpkgs-flake { };
  libOverlay = (import ../lib).overlay;
  lib = nixpkgs.lib.extend libOverlay;

  # collection tests
  collectionTests = {
    "test: non existing pkgs dir results in empty entries (no error)" = {
      expr = lib.nef.dirToAttrs "/does/not/exist";
      expected = {
        path = "/does/not/exist";
        type = "directory";
        entries = { };
      };
    };
    "test: empty pkgs dir results in empty entries (no error)" = {
      expr = lib.nef.dirToAttrs ./emptyPkgs;
      expected = {
        path = ./emptyPkgs;
        type = "directory";
        entries = { };
      };
    };

    "test: collects package with default nix" = {
      expr = (lib.nef.dirToAttrs ./pkgs).entries ? newPackage;
      expected = true;
    };

    "test: collects package with plain nix file" = {
      expr = (lib.nef.dirToAttrs ./pkgs).entries ? plainFile;
      expected = true;
    };

    "test: collecting does not eval" = {
      expr = builtins.deepSeq (lib.nef.dirToAttrs ./pkgs).entries.lazyEval true;
      expected = true;
    };

    "test: collects nested package" = {
      expr = (lib.nef.dirToAttrs ./pkgs).entries.setMakeScope.entries ? makeScopeDependency;
      expected = true;
    };
  };

  extensionTests =
    let
      collected = lib.nef.dirToAttrs ./pkgs;
      pkgs = lib.nef.extendAttrSet [ ] { } (import ./basePackageSet.nix { inherit lib; }) collected;
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
    };

in
{
  inherit collectionTests;
}
