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

  "test: <package>.nix takes precedence" =
    let
      root = lib.nef.dirToAttrs "${./testData/collectionPrecedence}";
      rootPath = root.path;
      fooPath = root.entries.foo.path;
    in
    {
      expr = fooPath;
      expected = "${rootPath}/foo.nix";
    };

  "test: <package>.nix shadows <package> dir" =
    let
      root = lib.nef.dirToAttrs "${./testData/collectionPrecedence}";
      rootPath = root.path;
      bar = root.entries.bar;
    in
    {
      expr = bar;
      expected = {
        path = "${rootPath}/bar.nix";
        type = "nix";
      };
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
