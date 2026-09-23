{ lib, nixpkgs }:
let
  instantiate = lib.nef.instantiate;

  # A fake, cheap-to-evaluate base package set in place of real nixpkgs.
  # `applyDeepOverrides` only requires its base to be extensible
  # (`overrideScope`/`extend`); it carries no nixpkgs-specific behavior.
  # `topLevelDependency` is this set's designated override target.
  basePkgs = import ./testData/basePackageSet.nix { inherit lib; };

  pathSource = dir: {
    type = "path";
    path = "${dir}";
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
in
{
  # Runs through real nixpkgs rather than the fake `basePkgs`, since
  # `instantiateFromSourceInfo` hard-codes `nixpkgs.extend` for its own
  # catalog overlay (`instantiate.nix`, pre-existing, unrelated to deep
  # overrides) and a `makeScope`-built fake set has no such method.
  # `nefDeepOverrideDemo` is a name real nixpkgs does not define, so
  # this stays clear of the base's own package derivations.
  "test: an override in a dependency's source reaches the consumer's build" =
    let
      deepOverrideTree = instantiate.collectDeepOverrides {
        catalogSpecClosure = singlePackageClosure "bar" ./testData/deepOverrides/dependency [
          "nefDeepOverrideDemo"
        ];
        sourceInfo = consumerSourceInfo;
      };
      nixpkgsWithDeepOverrides = instantiate.applyDeepOverrides nixpkgs deepOverrideTree;
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
    # consumer's own pkgs/, never in the deep overlay's scope; its
    # default value stands in, proving pkgs/ is not visible here.
    expected = "not visible to deep overrides";
  };

  "test: two sources overriding the same attribute path is a collision error" = {
    expr =
      let
        deepOverrideTree = instantiate.collectDeepOverrides {
          catalogSpecClosure = {
            depA =
              (singlePackageClosure "bar" ./testData/deepOverrides/dependency [
                "topLevelDependency"
              ]).dep;
            depB =
              (singlePackageClosure "baz" ./testData/deepOverrides/sibling [
                "topLevelDependency"
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

  # `setMakeScope` is a nested package set in `basePackageSet.nix`;
  # its override lives at `__overrides/setMakeScope/makeScopeDependency.nix`
  # in the dependency fixture, proving the target attr path is read off
  # the directory structure below `__overrides`, not just its top level.
  "test: a deep override nested under a package set reaches that nested attribute" = {
    expr =
      let
        deepOverrideTree = instantiate.collectDeepOverrides {
          catalogSpecClosure = singlePackageClosure "baz" ./testData/deepOverrides/dependency [
            "setMakeScope.makeScopeDependency"
          ];
          sourceInfo = consumerSourceInfo;
        };
      in
      (instantiate.applyDeepOverrides basePkgs deepOverrideTree).setMakeScope.makeScopeDependency;
    expected = "overridden nested by dependency";
  };

  # `__overrides` (see the reserved-name comment on `overridesDirName`
  # in instantiate.nix) sits inside the consumer's own `pkgs/`, next to
  # `usesDeepOverrideTarget.nix` and `localTool.nix`; this checks that
  # `instantiateFromSourceInfo` never turns it into a package set of
  # its own, alongside the ordinary attrs it does expose.
  "test: __overrides is excluded from a repository's own package tree" =
    let
      deepOverrideTree = instantiate.collectDeepOverrides {
        catalogSpecClosure = { };
        sourceInfo = consumerSourceInfo;
      };
      instantiated = instantiate.instantiateFromSourceInfo {
        nixpkgs = instantiate.applyDeepOverrides nixpkgs deepOverrideTree;
        sourceInfo = consumerSourceInfo;
        instantiatedCatalogsClosure = { };
      };
    in
    {
      expr = {
        hasOverridesAttr = instantiated.pkgs ? __overrides;
        hasOrdinaryAttrs = instantiated.pkgs ? localTool && instantiated.pkgs ? usesDeepOverrideTarget;
      };
      expected = {
        hasOverridesAttr = false;
        hasOrdinaryAttrs = true;
      };
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
}
