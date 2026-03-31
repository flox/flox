{
  lib,
  nixpkgs,
  fixtures,
}:
let
  instantiate = lib.nef.instantiate;

  singleLevel = instantiate.instantiateFromSourceInfo {
    inherit nixpkgs;
    sourceInfo = builtins.fetchTree "path:${fixtures}/single-level/root";
  };

  multiLevel = instantiate.instantiateFromSourceInfo {
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

  # FloxHub catalog tests

  "test: floxhub catalog instantiates package" =
    let
      result = instantiate.instantiateCatalog nixpkgs {
        type = "floxhub";
        packages = {
          type = "package_set";
          entries = {
            dep = {
              type = "package";
              build_type = "nef";
              source = {
                type = "path";
                path = "${fixtures}/single-level/child";
              };
            };
          };
        };
      };
    in
    {
      expr = result.packages.dep;
      expected = "i am dep";
    };

  "test: floxhub catalog nested package set" =
    let
      result = instantiate.instantiateCatalog nixpkgs {
        type = "floxhub";
        packages = {
          type = "package_set";
          entries = {
            nested = {
              type = "package_set";
              entries = {
                deep = {
                  type = "package";
                  build_type = "nef";
                  source = {
                    type = "path";
                    path = "${fixtures}/single-level/child";
                  };
                };
              };
            };
          };
        };
      };
    in
    {
      expr = result.packages.nested.deep;
      expected = "i am nested";
    };

  "test: floxhub catalog type is preserved" =
    let
      result = instantiate.instantiateCatalog nixpkgs {
        type = "floxhub";
        packages = {
          type = "package_set";
          entries = { };
        };
      };
    in
    {
      expr = result.type;
      expected = "floxhub";
    };
}
