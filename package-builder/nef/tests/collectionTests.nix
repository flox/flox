{ lib }:
# collection tests
{
  "test: non existing pkgs dir results in empty entries (no error)" = {
    expr = lib.nef.dirToAttrs "/does/not/exist";
    expected = {
      path = "/does/not/exist";
      type = "directory";
      entries = { };
    };
  };
  "test: empty pkgs dir results in empty entries (no error)" = {
    expr = lib.nef.dirToAttrs ./testData/emptyPkgs;
    expected = {
      path = ./testData/emptyPkgs;
      type = "directory";
      entries = { };
    };
  };

  "test: collects package with default nix" = {
    expr = (lib.nef.dirToAttrs ./testData/pkgs).entries ? newPackage;
    expected = true;
  };

  "test: collects package with plain nix file" = {
    expr = (lib.nef.dirToAttrs ./testData/pkgs).entries ? plainFile;
    expected = true;
  };

  "test: collecting does not eval" = {
    expr = builtins.deepSeq (lib.nef.dirToAttrs ./testData/pkgs).entries.lazyEval true;
    expected = true;
  };

  "test: collects nested package" = {
    expr = (lib.nef.dirToAttrs ./testData/pkgs).entries.setMakeScope.entries ? makeScopeDependency;
    expected = true;
  };
}
