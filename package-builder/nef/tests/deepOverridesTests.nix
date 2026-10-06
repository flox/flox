{ lib }:
let
  instantiate = lib.nef.instantiate;

  # A fake, cheap-to-evaluate base package set in place of real nixpkgs.
  # `applyDeepOverrides` only requires its base to be extensible
  # (`overrideScope`/`extend`); it carries no nixpkgs-specific behavior.
  # `topLevelDependency` is this set's designated override target.
  basePkgs = import ./testData/basePackageSet.nix { inherit lib; };

  # Stands in for top-level nixpkgs in tests that run the whole pipeline.
  # `instantiateFromSourceInfo` needs `.extend`, which `basePkgs` lacks.
  topLevelLikePkgs = import ./testData/topLevelLikePackageSet.nix { inherit lib; };

  pathSource = dir: {
    type = "path";
    path = "${dir}";
  };

  # A path source for one of several package directories in the same
  # repository. `root` is the fetched tree and `subdir` its `dir`.
  pathSourceInDir = root: subdir: {
    type = "path";
    path = "${root}";
    dir = subdir;
  };

  # A one-catalog closure with a single package at `attrPath`, sourced
  # from `dir`, flagged with `deepOverrides`.
  singlePackageClosure = attrPath: dir: deepOverrides: {
    dep = {
      type = "floxhub";
      packages = {
        type = "package_set";
        entries = {
          ${attrPath} = {
            type = "package";
            build_type = "nef";
            source = pathSource dir;
            deep_overrides = deepOverrides;
          };
        };
      };
    };
  };

  consumerSourceInfo = {
    outPath = "${./testData/deepOverrides/consumer}";
  };

  # A consuming project with a shallow `zlib.nix` and no deep overrides.
  # `consumerSourceInfo` cannot pair with `topLevelLikePkgs`: its deep
  # overrides target names that set does not define.
  consumerWithShallowZlibSourceInfo = {
    outPath = "${./testData/deepOverrides/consumerWithShallowZlib}";
  };
in
{
  "test: an override in a dependency's source reaches the consumer's build" =
    let
      deepOverrideTree = instantiate.collectDeepOverrides {
        catalogSpecClosure = singlePackageClosure "bar" ./testData/deepOverrides/dependency [
          [ "topLevelDependency" ]
        ];
        sourceInfo = consumerSourceInfo;
      };
      nixpkgsWithDeepOverrides = instantiate.applyDeepOverrides topLevelLikePkgs deepOverrideTree;
      instantiated = instantiate.instantiateFromSourceInfo {
        nixpkgs = nixpkgsWithDeepOverrides;
        sourceInfo = consumerSourceInfo;
        instantiatedCatalogsClosure = { };
      };
    in
    {
      # The consumer's own `usesDeepOverrideTarget.nix` never mentions
      # `dep.bar` itself: depending on the package is what pulls its
      # repository's deep override into the shared base, ahead of the
      # consumer's own `pkgs/` being applied.
      expr = instantiated.pkgs.usesDeepOverrideTarget;
      expected = "consumer sees: overridden by dependency";
    };

  "test: an override that forces catalogs fails, naming why" =
    let
      deepOverrideTree = instantiate.collectDeepOverrides {
        catalogSpecClosure = singlePackageClosure "bar" ./testData/deepOverrides/dependency [
          [ "catalogSeeker" ]
        ];
        sourceInfo = consumerSourceInfo;
      };
      nixpkgsWithDeepOverrides = instantiate.applyDeepOverrides topLevelLikePkgs deepOverrideTree;
      instantiated = instantiate.instantiateFromSourceInfo {
        nixpkgs = nixpkgsWithDeepOverrides;
        sourceInfo = consumerSourceInfo;
        instantiatedCatalogsClosure = { };
      };
    in
    {
      expr = instantiated.pkgs.catalogSeeker;
      expectedError = {
        type = "ThrownError";
        msg = "cannot use catalog packages";
      };
    };

  "test: a deep override does not see its own repository's pkgs/ tree" = {
    expr =
      let
        deepOverrideTree = instantiate.collectDeepOverrides {
          catalogSpecClosure = { };
          sourceInfo = consumerSourceInfo;
        };
      in
      (instantiate.applyDeepOverrides basePkgs deepOverrideTree).usesLocalTool;
    # `usesLocalTool.nix` requests `localTool`, which exists only in the
    # consumer's own pkgs/, never in the deep overlay's scope: a clear
    # throw naming the problem, not a silent fallback to its default.
    expectedError = {
      type = "ThrownError";
      msg = "not a deep override";
    };
  };

  "test: override cannot reach catalogs by calling a project package instead" = {
    expr =
      let
        deepOverrideTree = instantiate.collectDeepOverrides {
          catalogSpecClosure = { };
          sourceInfo = consumerSourceInfo;
        };
      in
      (instantiate.applyDeepOverrides basePkgs deepOverrideTree).reachesProjectPkg;
    expectedError = {
      type = "ThrownError";
      msg = "not a deep override";
    };
  };

  "test: two sources overriding the same attribute path is a collision error" = {
    expr =
      let
        deepOverrideTree = instantiate.collectDeepOverrides {
          catalogSpecClosure = {
            depA =
              (singlePackageClosure "bar" ./testData/deepOverrides/dependency [
                [ "topLevelDependency" ]
              ]).dep;
            depB =
              (singlePackageClosure "baz" ./testData/deepOverrides/sibling [
                [ "topLevelDependency" ]
              ]).dep;
          };
          sourceInfo = consumerSourceInfo;
        };
      in
      (instantiate.applyDeepOverrides basePkgs deepOverrideTree).topLevelDependency;
    expectedError = {
      type = "ThrownError";
      msg = "Deep override collision on 'topLevelDependency'";
    };
  };

  # `basePkgs` has no `absentTopLevel`.
  "test: an override of an absent top-level name throws, naming it" = {
    expr =
      let
        deepOverrideTree = instantiate.collectDeepOverrides {
          catalogSpecClosure = singlePackageClosure "bar" ./testData/deepOverrides/absentTargets [
            [ "absentTopLevel" ]
          ];
          sourceInfo = consumerSourceInfo;
        };
      in
      (instantiate.applyDeepOverrides basePkgs deepOverrideTree).absentTopLevel;
    expectedError = {
      type = "ThrownError";
      msg = "Only packages that already exist in the base can be deeply overridden";
    };
  };

  # `basePkgs` has no `absentNestedSet`, so there is no set to extend.
  "test: an override of an absent nested package set throws the same way" = {
    expr =
      let
        deepOverrideTree = instantiate.collectDeepOverrides {
          catalogSpecClosure = singlePackageClosure "bar" ./testData/deepOverrides/absentTargets [
            [
              "absentNestedSet"
              "absentChild"
            ]
          ];
          sourceInfo = consumerSourceInfo;
        };
      in
      (instantiate.applyDeepOverrides basePkgs deepOverrideTree).absentNestedSet.absentChild;
    expectedError = {
      type = "ThrownError";
      msg = "Only packages that already exist in the base can be deeply overridden";
    };
  };

  # Two locked sources share a `path` and differ only in `dir`. If
  # their labels collided, one source would be fetched with the other's
  # `dir`, and its locked override would fail the lock check.
  "test: two locked sources from the same path with different dir both apply" = {
    expr =
      let
        sameSourceDifferentDirs = ./testData/deepOverrides/sameSourceDifferentDirs;
        deepOverrideTree = instantiate.collectDeepOverrides {
          catalogSpecClosure = {
            depA = {
              type = "floxhub";
              packages = {
                type = "package_set";
                entries = {
                  overrideA = {
                    type = "package";
                    build_type = "nef";
                    source = pathSourceInDir sameSourceDifferentDirs "dirA";
                    deep_overrides = [ [ "overrideA" ] ];
                  };
                };
              };
            };
            depB = {
              type = "floxhub";
              packages = {
                type = "package_set";
                entries = {
                  overrideB = {
                    type = "package";
                    build_type = "nef";
                    source = pathSourceInDir sameSourceDifferentDirs "dirB";
                    deep_overrides = [ [ "overrideB" ] ];
                  };
                };
              };
            };
          };
          sourceInfo = consumerSourceInfo;
        };
        applied = instantiate.applyDeepOverrides basePkgs deepOverrideTree;
      in
      {
        overrideA = applied.overrideA;
        overrideB = applied.overrideB;
      };
    expected = {
      overrideA = "dirA override sees value";
      overrideB = "dirB override sees value";
    };
  };

  # `setMakeScope` is a nested package set in `basePackageSet.nix`;
  # its override lives at `setMakeScope/makeScopeDependency/deep-override`
  # in the dependency fixture, proving a nested marker targets the
  # attribute at its own position in the tree, not just a top-level one.
  "test: a deep override nested under a package set reaches that nested attribute" = {
    expr =
      let
        deepOverrideTree = instantiate.collectDeepOverrides {
          catalogSpecClosure = singlePackageClosure "baz" ./testData/deepOverrides/dependency [
            [
              "setMakeScope"
              "makeScopeDependency"
            ]
          ];
          sourceInfo = consumerSourceInfo;
        };
      in
      (instantiate.applyDeepOverrides basePkgs deepOverrideTree).setMakeScope.makeScopeDependency;
    expected = "overridden nested by dependency";
  };

  # `dependency` deep-overrides `topLevelDependency`.
  # `depShadowsOverrideTarget` has a shallow package of the same name,
  # which gets a stub. Its override must see the deep override, not
  # the stub.
  "test: a source's own shallow sibling does not shadow another source's override of the same name" =
    {
      expr =
        let
          deepOverrideTree = instantiate.collectDeepOverrides {
            catalogSpecClosure = {
              depT =
                (singlePackageClosure "topLevelOverride" ./testData/deepOverrides/dependency [
                  [ "topLevelDependency" ]
                ]).dep;
              depS =
                (singlePackageClosure "usesTopLevelDependency" ./testData/deepOverrides/depShadowsOverrideTarget [
                  [ "usesTopLevelDependency" ]
                ]).dep;
            };
            sourceInfo = consumerSourceInfo;
          };
        in
        (instantiate.applyDeepOverrides basePkgs deepOverrideTree).usesTopLevelDependency;
      expected = "depShadowsOverrideTarget sees overridden by dependency";
    };

  # `setMakeExtensible` has no `newScope` or `callPackage`, so its
  # packages go through `callPackageIn`'s fallback branch. In that branch
  # the set's own `prev` outranks `currentScope`. The base value of
  # `extensibleDependency` throws, so the test fails if the override
  # loses.
  "test: inside a makeExtensible-only nested set, a sibling override wins over the set's own pre-overlay value" =
    let
      deepOverrideTree = instantiate.collectDeepOverrides {
        catalogSpecClosure =
          singlePackageClosure "thing" ./testData/deepOverrides/depOverridesNestedExtensibleSiblings
            [
              [
                "setMakeExtensible"
                "extensibleDependency"
              ]
              [
                "setMakeExtensible"
                "extensibleDependent"
              ]
            ];
        sourceInfo = consumerSourceInfo;
      };
    in
    {
      expr =
        (instantiate.applyDeepOverrides basePkgs deepOverrideTree).setMakeExtensible.extensibleDependent;
      expected = "extensibleDependent sees overridden nested in makeExtensible";
    };

  # `reachesProjectPkg` and `usesLocalTool`, both marked with a
  # `deep-override` file next to their `default.nix`, sit inside the
  # consumer's own `pkgs/`, alongside unmarked `usesDeepOverrideTarget.nix`
  # and `localTool.nix`. The override is already baked into `pkgs`
  # itself via the base, so this checks the repository's own reflected
  # attrPaths instead: `instantiateFromSourceInfo` must never list a
  # marked entry as one of its own packages, alongside the shallow
  # ones it does list.
  "test: a marked package is excluded from a repository's own package tree" =
    let
      deepOverrideTree = instantiate.collectDeepOverrides {
        catalogSpecClosure = { };
        sourceInfo = consumerSourceInfo;
      };
      instantiated = instantiate.instantiateFromSourceInfo {
        nixpkgs = instantiate.applyDeepOverrides topLevelLikePkgs deepOverrideTree;
        sourceInfo = consumerSourceInfo;
        instantiatedCatalogsClosure = { };
      };
      reflectedNames = map (p: p.attrPathStr) instantiated.reflect.attrPaths;
    in
    {
      expr = {
        hasMarkedAttrs = lib.any (n: builtins.elem n reflectedNames) [
          "reachesProjectPkg"
          "usesLocalTool"
        ];
        hasShallowAttrs = lib.all (n: builtins.elem n reflectedNames) [
          "localTool"
          "usesDeepOverrideTarget"
        ];
      };
      expected = {
        hasMarkedAttrs = false;
        hasShallowAttrs = true;
      };
    };

  # `notAnOverride.nix` in the dependency fixture has no `deep-override`
  # marker, so it is a shallow package of that repository, never
  # folded into the base: a sibling cannot see it through the base the
  # way it sees an actual deep override.
  "test: an unmarked package is shallow and is not applied to the base" =
    let
      deepOverrideTree = instantiate.collectDeepOverrides {
        catalogSpecClosure = singlePackageClosure "bar" ./testData/deepOverrides/dependency [
          [ "topLevelDependency" ]
        ];
        sourceInfo = consumerSourceInfo;
      };
    in
    {
      expr = (instantiate.applyDeepOverrides basePkgs deepOverrideTree) ? notAnOverride;
      expected = false;
    };

  "test: a locked source without deep_overrides is never fetched" =
    let
      # Both entries point at paths that do not exist; if either were
      # fetched, this test would fail with a fetch error rather than
      # the expected value below.
      catalogSpecClosure = {
        dep = {
          type = "floxhub";
          packages = {
            type = "package_set";
            entries = {
              # Explicit empty list.
              bar = {
                type = "package";
                build_type = "nef";
                source = pathSource "/does/not/exist/unfetchable-a";
                deep_overrides = [ ];
              };
              # Key absent entirely, as on a lock predating this field.
              baz = {
                type = "package";
                build_type = "nef";
                source = pathSource "/does/not/exist/unfetchable-b";
              };
            };
          };
        };
      };
      deepOverrideTree = instantiate.collectDeepOverrides {
        inherit catalogSpecClosure;
        sourceInfo = consumerSourceInfo;
      };
    in
    {
      expr = (instantiate.applyDeepOverrides basePkgs deepOverrideTree).topLevelValue;
      expected = "value";
    };

  # `dependency`'s `pkgs/` tree is real (fetched successfully), but it
  # has no `doesNotExist/default.nix`: the lock and the source it
  # names disagree, which is an error naming both rather than a
  # silently skipped override. Forcing an unrelated attribute
  # (`topLevelValue`) is enough to surface it -- the check runs for
  # every locked pair the moment the override tree is grouped, not
  # only for whichever one is actually evaluated.
  "test: a locked attrPath the fetched source does not define is a disagreement error" = {
    expr =
      let
        deepOverrideTree = instantiate.collectDeepOverrides {
          catalogSpecClosure = singlePackageClosure "bar" ./testData/deepOverrides/dependency [
            [ "doesNotExist" ]
          ];
          sourceInfo = consumerSourceInfo;
        };
      in
      (instantiate.applyDeepOverrides basePkgs deepOverrideTree).topLevelValue;
    expectedError = {
      type = "ThrownError";
      msg = "disagreement";
    };
  };

  "test: unmarked sibling coinciding with a real upstream name warns and uses it" =
    let
      deepOverrideTree = instantiate.collectDeepOverrides {
        catalogSpecClosure = singlePackageClosure "thing" ./testData/deepOverrides/depWithTopLevelSibling [
          [ "wantsTopLevelSibling" ]
        ];
        sourceInfo = consumerSourceInfo;
      };
    in
    {
      expr = (instantiate.applyDeepOverrides basePkgs deepOverrideTree).wantsTopLevelSibling;
      expected = "value";
    };

  "test: unmarked nested sibling coinciding with the nested set upstream warns and uses it" =
    let
      deepOverrideTree = instantiate.collectDeepOverrides {
        catalogSpecClosure = singlePackageClosure "thing" ./testData/deepOverrides/depWithNestedSiblings [
          [
            "setMakeScope"
            "wantsExistingNestedSibling"
          ]
        ];
        sourceInfo = consumerSourceInfo;
      };
    in
    {
      expr =
        (instantiate.applyDeepOverrides basePkgs deepOverrideTree).setMakeScope.wantsExistingNestedSibling;
      expected = "value";
    };

  "test: unmarked nested sibling absent from the nested set upstream throws" =
    let
      deepOverrideTree = instantiate.collectDeepOverrides {
        catalogSpecClosure = singlePackageClosure "thing" ./testData/deepOverrides/depWithNestedSiblings [
          [
            "setMakeScope"
            "wantsMissingNestedSibling"
          ]
        ];
        sourceInfo = consumerSourceInfo;
      };
    in
    {
      expr =
        (instantiate.applyDeepOverrides basePkgs deepOverrideTree).setMakeScope.wantsMissingNestedSibling;
      expectedError = {
        type = "ThrownError";
        msg = "not a deep override";
      };
    };

  # Two sources, each with its own override and its own unmarked
  # sibling. Proves sibling stubs are scoped per source:
  # depAskingForOtherSibling's "wantsOtherSourceSibling" asks for a
  # name only depWithOwnSibling defines as a sibling, and gets its own
  # default rather than depWithOwnSibling's throw stub -- while
  # depAskingForOtherSibling's *own* sibling still throws for its own
  # override below, proving each source is independently correct, not
  # just "one leaks, the other doesn't".
  "test: an override's unmarked siblings do not leak into another source's override" =
    let
      crossSourceCatalogSpecClosure = {
        a =
          (singlePackageClosure "a" ./testData/deepOverrides/depWithOwnSibling [
            [ "wantsOwnSibling" ]
          ]).dep;
        b =
          (singlePackageClosure "b" ./testData/deepOverrides/depAskingForOtherSibling [
            [ "wantsOtherSourceSibling" ]
            [ "wantsOtherOwnSibling" ]
          ]).dep;
      };
      deepOverrideTree = instantiate.collectDeepOverrides {
        catalogSpecClosure = crossSourceCatalogSpecClosure;
        sourceInfo = consumerSourceInfo;
      };
    in
    {
      expr = (instantiate.applyDeepOverrides basePkgs deepOverrideTree).wantsOtherSourceSibling;
      expected = "source B does not see source A's sibling";
    };

  "test: a source's own unmarked sibling still throws alongside another source's override" =
    let
      crossSourceCatalogSpecClosure = {
        a =
          (singlePackageClosure "a" ./testData/deepOverrides/depWithOwnSibling [
            [ "wantsOwnSibling" ]
          ]).dep;
        b =
          (singlePackageClosure "b" ./testData/deepOverrides/depAskingForOtherSibling [
            [ "wantsOtherSourceSibling" ]
            [ "wantsOtherOwnSibling" ]
          ]).dep;
      };
      deepOverrideTree = instantiate.collectDeepOverrides {
        catalogSpecClosure = crossSourceCatalogSpecClosure;
        sourceInfo = consumerSourceInfo;
      };
    in
    {
      expr = (instantiate.applyDeepOverrides basePkgs deepOverrideTree).wantsOwnSibling;
      expectedError = {
        type = "ThrownError";
        msg = "not a deep override";
      };
    };

  "test: a second source's own unmarked sibling independently throws too" =
    let
      crossSourceCatalogSpecClosure = {
        a =
          (singlePackageClosure "a" ./testData/deepOverrides/depWithOwnSibling [
            [ "wantsOwnSibling" ]
          ]).dep;
        b =
          (singlePackageClosure "b" ./testData/deepOverrides/depAskingForOtherSibling [
            [ "wantsOtherSourceSibling" ]
            [ "wantsOtherOwnSibling" ]
          ]).dep;
      };
      deepOverrideTree = instantiate.collectDeepOverrides {
        catalogSpecClosure = crossSourceCatalogSpecClosure;
        sourceInfo = consumerSourceInfo;
      };
    in
    {
      expr = (instantiate.applyDeepOverrides basePkgs deepOverrideTree).wantsOtherOwnSibling;
      expectedError = {
        type = "ThrownError";
        msg = "not a deep override";
      };
    };

  # `dep` deep-overrides `zlib` and `hello`, and `hello` takes `zlib`.
  # The consuming project has a shallow `zlib.nix`. In the project's
  # namespace `hello` gets the project's `zlib`. Elsewhere it gets
  # `dep`'s.
  "test: a namespace's shallow package reaches a deep override evaluated in that namespace" =
    let
      deepOverrideTree = instantiate.collectDeepOverrides {
        catalogSpecClosure =
          singlePackageClosure "helloUser" ./testData/deepOverrides/depDeepOverridesZlibHello
            [
              [ "zlib" ]
              [ "hello" ]
            ];
        sourceInfo = consumerWithShallowZlibSourceInfo;
      };
      nixpkgsWithDeepOverrides = instantiate.applyDeepOverrides topLevelLikePkgs deepOverrideTree;
      instantiated = instantiate.instantiateFromSourceInfo {
        nixpkgs = nixpkgsWithDeepOverrides;
        sourceInfo = consumerWithShallowZlibSourceInfo;
        instantiatedCatalogsClosure = { };
      };
    in
    {
      expr = {
        base = nixpkgsWithDeepOverrides.hello;
        namespaced = instantiated.pkgs.hello;
      };
      expected = {
        base = "hello built with dep's deep zlib";
        namespaced = "hello built with project wraps: dep's deep zlib";
      };
    };

  # `catalogs.dep.helloUser` is built in `dep`'s own namespace, which
  # does not include the consuming project. It gets `dep`'s deep `zlib`.
  "test: a catalog source's packages still get the dependency's deep override, not the consuming project's shallow package" =
    let
      catalogSpecClosure =
        singlePackageClosure "helloUser" ./testData/deepOverrides/depDeepOverridesZlibHello
          [
            [ "zlib" ]
            [ "hello" ]
          ];
      deepOverrideTree = instantiate.collectDeepOverrides {
        inherit catalogSpecClosure;
        sourceInfo = consumerWithShallowZlibSourceInfo;
      };
      nixpkgsWithDeepOverrides = instantiate.applyDeepOverrides topLevelLikePkgs deepOverrideTree;
      instantiatedCatalogs = instantiate.instantiateCatalogs {
        nixpkgs = nixpkgsWithDeepOverrides;
        inherit catalogSpecClosure;
      };
    in
    {
      expr = instantiatedCatalogs.dep.packages.helloUser;
      expected = "dep catalog sees: hello built with dep's deep zlib";
    };

  # `nested` has no `newScope` or `callPackage`, so `nested.catalogSeeker`
  # goes through `callPackageIn`'s fallback branch. In the project's
  # namespace `final` has a real `catalogs`. The denial must still win.
  "test: inside the consuming project's namespace, a deep override requesting catalogs still throws the denial" =
    let
      deepOverrideTree = instantiate.collectDeepOverrides {
        catalogSpecClosure = singlePackageClosure "bar" ./testData/deepOverrides/depRequestsCatalogsNested [
          [
            "nested"
            "catalogSeeker"
          ]
        ];
        sourceInfo = consumerWithShallowZlibSourceInfo;
      };
      nixpkgsWithDeepOverrides = instantiate.applyDeepOverrides topLevelLikePkgs deepOverrideTree;
      instantiated = instantiate.instantiateFromSourceInfo {
        nixpkgs = nixpkgsWithDeepOverrides;
        sourceInfo = consumerWithShallowZlibSourceInfo;
        instantiatedCatalogsClosure = { };
      };
    in
    {
      expr = instantiated.pkgs.nested.catalogSeeker;
      expectedError = {
        type = "ThrownError";
        msg = "cannot use catalog packages";
      };
    };
}
