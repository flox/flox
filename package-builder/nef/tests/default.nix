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

in
{
  inherit collectionTests;
}
