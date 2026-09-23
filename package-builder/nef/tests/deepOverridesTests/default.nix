{ lib }:
let
  instantiate = lib.nef.instantiate;
  basePkgs = import ./testData/basePkgs.nix { inherit lib; };

  # One catalog with three locked packages under different sources.
  # Only `bar` is referenced from `consumer/pkgs/useBar.nix`; `baz` and
  # `qux` exercise "arrives unasked" (FLO-95 design: deep overrides
  # reach the base whether or not the consumer references the package
  # that carries them) and the eager-fetch cost of discovering
  # overrides from every locked source, not just referenced ones.
  mainCatalogSpecClosure = {
    acme = {
      type = "floxhub";
      packages = {
        type = "package_set";
        entries = {
          bar = {
            type = "package";
            build_type = "nef";
            source = {
              type = "path";
              path = toString ./testData/dep;
            };
          };
          baz = {
            type = "package";
            build_type = "nef";
            source = {
              type = "path";
              path = toString ./testData/depOther;
            };
          };
          qux = {
            type = "package";
            build_type = "nef";
            source = {
              type = "path";
              path = toString ./testData/depThird;
            };
          };
        };
      };
    };
  };

  consumerSourceInfo = {
    outPath = toString ./testData/consumer;
  };

  mainDeepOverrideTree = instantiate.collectDeepOverrides {
    catalogSpecClosure = mainCatalogSpecClosure;
    selfSourceInfo = consumerSourceInfo;
  };

  nixpkgsWithOverrides = instantiate.applyDeepOverrides basePkgs mainDeepOverrideTree;

  instantiatedCatalogsClosure = instantiate.instantiateCatalogs {
    nixpkgs = nixpkgsWithOverrides;
    catalogSpecClosure = mainCatalogSpecClosure;
  };

  consumer = instantiate.instantiateFromSourceInfo {
    nixpkgs = nixpkgsWithOverrides;
    sourceInfo = consumerSourceInfo;
    inherit instantiatedCatalogsClosure;
  };

  # A catalog whose one package's source has an override that requests
  # its own repository's `pkgs/` sibling by name.
  siblingCatalogSpecClosure = {
    acme = {
      type = "floxhub";
      packages = {
        type = "package_set";
        entries = {
          thing = {
            type = "package";
            build_type = "nef";
            source = {
              type = "path";
              path = toString ./testData/depWithSiblingPkgs;
            };
          };
        };
      };
    };
  };

  siblingDeepOverrideTree = instantiate.collectDeepOverrides {
    catalogSpecClosure = siblingCatalogSpecClosure;
    selfSourceInfo = consumerSourceInfo;
  };

  nixpkgsForSibling = instantiate.applyDeepOverrides basePkgs siblingDeepOverrideTree;

  # Two different locked sources, each overriding the same attribute.
  collisionCatalogSpecClosure = {
    acme = {
      type = "floxhub";
      packages = {
        type = "package_set";
        entries = {
          a = {
            type = "package";
            build_type = "nef";
            source = {
              type = "path";
              path = toString ./testData/collisionA;
            };
          };
          b = {
            type = "package";
            build_type = "nef";
            source = {
              type = "path";
              path = toString ./testData/collisionB;
            };
          };
        };
      };
    };
  };
in
{
  "test: override in a dependency's source reaches the consumer's build" = {
    expr = consumer.pkgs.useBar;
    expected = "bar says overridden greeting";
  };

  "test: override lands in the base even when its own package is unreferenced" = {
    expr = nixpkgsWithOverrides.greeting;
    expected = "overridden greeting";
  };

  "test: nested override path targets the nested attribute" = {
    expr = nixpkgsWithOverrides.python3Packages.foo;
    expected = "overridden foo";
  };

  "test: consuming project's own pkgs/__overrides is applied to the base" = {
    expr = nixpkgsWithOverrides.onlyForBase;
    expected = "only for base";
  };

  "test: an override that forces catalogs fails, naming why" = {
    expr = nixpkgsWithOverrides.wantsCatalogs;
    expectedError = {
      type = "ThrownError";
      msg = "cannot use catalog packages";
    };
  };

  # `onlyForBase` above is defined in the same consuming project's
  # `pkgs/__overrides` and never references `catalogs`, so its
  # passing test above already covers an override left unaffected.

  "test: __overrides does not surface as a package set in the repository's own tree" = {
    expr = builtins.hasAttr "__overrides" consumer.pkgs;
    expected = false;
  };

  "test: override does not see its own repository's sibling pkgs" = {
    expr = nixpkgsForSibling.wantsSibling;
    expectedError = {
      type = "Abort";
      msg = "sibling";
    };
  };

  "test: two sources overriding the same attribute is a collision error naming both" = {
    expr =
      let
        collisionDeepOverrideTree = instantiate.collectDeepOverrides {
          catalogSpecClosure = collisionCatalogSpecClosure;
          selfSourceInfo = consumerSourceInfo;
        };
      in
      (instantiate.applyDeepOverrides basePkgs collisionDeepOverrideTree).greeting;
    expectedError = {
      type = "ThrownError";
      msg = "collision";
    };
  };
}
