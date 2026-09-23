{ lib }:
let
  fetchSource =
    source:
    let
      lockedWithoutDir = builtins.removeAttrs source [ "dir" ];
      sourceInfo = builtins.addErrorContext "while fetching source '${builtins.flakeRefToString lockedWithoutDir}'" (
        builtins.fetchTree lockedWithoutDir
      );
    in
    sourceInfo // lib.optionalAttrs (source ? dir) { inherit (source) dir; };

  # A human-readable label for a locked source, reused as the identity
  # in deep-override collision errors below.
  labelSource = source: builtins.flakeRefToString (builtins.removeAttrs source [ "dir" ]);

  # NEF's one reserved name: a `pkgs/__overrides` directory holds deep
  # overrides rather than ordinary packages. It sits inside `pkgs/` so
  # `__overrides.openssl` is a publishable attr path (design note,
  # "Splitting the package tree"), which means `instantiateFromSourceInfo`
  # below must strip it out before it reaches a repository's own package
  # tree, exactly once, at the point `pkgs/` is collected.
  overridesDirName = "__overrides";

  # Every locked package source in a catalog's package tree whose entry
  # records at least one deep override, i.e. `deep_overrides` is
  # present and non-empty (absent on locks predating the field, per
  # `PackageTreeNode::Package` in `nef-lock-catalog`). Mirrors the
  # "package" / "package_set" recursion `fetchFloxHubCatalog` below
  # uses to instantiate the same tree, but collects sources instead of
  # instantiating them, and only where flagged, so a project with no
  # deep overrides in its closure fetches nothing extra.
  collectDeepOverrideSources =
    node:
    {
      "package" = lib.optional ((node.deep_overrides or [ ]) != [ ]) node.source;
      "package_set" = lib.concatMap collectDeepOverrideSources (lib.attrValues node.entries);
    }
    .${node.type};

  # Every flagged source reachable from a catalog closure, one entry
  # per catalog. `nix` catalogs throw here exactly as `instantiateCatalog`
  # does, since they would fail on the same closure moments later.
  collectClosureDeepOverrideSources =
    catalogSpecClosure:
    lib.concatMap (
      catalogSpec:
      {
        "nix" = throw "source inputs not currently supported";
        "floxhub" = collectDeepOverrideSources catalogSpec.packages;
      }
      .${catalogSpec.type}
    ) (lib.attrValues catalogSpecClosure);

  # Merge the `pkgs/__overrides` trees of several sources (as produced by
  # `lib.nef.dirToAttrs`) into one tree of the same shape. Two sources
  # contributing the same attribute path is an evaluation error naming
  # both; two sources contributing different attributes under the same
  # subdirectory merge, since neither actually collides.
  mergeOverrideTrees =
    attrPath: labeledTrees:
    let
      allEntries = lib.concatMap (
        labeled:
        lib.mapAttrsToList (name: value: {
          inherit (labeled) label;
          inherit name value;
        }) labeled.tree.entries
      ) labeledTrees;
      grouped = lib.groupBy (entry: entry.name) allEntries;
    in
    lib.mapAttrs (
      name: group:
      if builtins.length group == 1 then
        (builtins.head group).value
      else if lib.all (entry: entry.value.type == "directory") group then
        {
          type = "directory";
          path = "<merged overrides at '${lib.showAttrPath (attrPath ++ [ name ])}'>";
          entries = mergeOverrideTrees (attrPath ++ [ name ]) (
            map (entry: {
              inherit (entry) label;
              tree = entry.value;
            }) group
          );
        }
      else
        throw ''
          Deep override collision on '${lib.showAttrPath (attrPath ++ [ name ])}': defined by both
          '${(builtins.elemAt group 0).label}' and '${(builtins.elemAt group 1).label}'.
        ''
    ) grouped;

  # Fetch a floxhub based catalog
  #
  # {
  #  packages = {
  #    hello = {
  #      source = {
  #        ref = "refs/heads/main";
  #        rev = "b59e1a5750b5714c88fb6a7f3232398107704f7b";
  #        type = "git";
  #        url = "https://github.com/flox/flox";
  #      };
  #      type = "package";
  #    };
  #    type = "package_set";
  #  };
  #  type = "floxhub";
  # };
  fetchFloxHubCatalog =
    nixpkgs: instantiatedCatalogsClosure: lockedCatalogSpec:
    let
      # process a package node
      processPackageNode =
        path: lockedPackageSpec:
        {
          "nef" =
            let
              sourceInfo = fetchSource lockedPackageSpec.source;
              instantiatedCatalog = lib.nef.instantiate.instantiateFromSourceInfo {
                inherit nixpkgs sourceInfo instantiatedCatalogsClosure;
              };
              instantiatedPackage = lib.getAttrFromPath path instantiatedCatalog.reflect.packages;
            in
            # Return the instantiated environment
            # The catalog overlay will use .reflect.packages
            instantiatedPackage;
          "manifest" = throw "Manifest build type not supported in Nix expressions";
        }
        .${lockedPackageSpec.build_type};

      # recurse into a package set
      processPackageSetNode =
        path: node: lib.mapAttrs (name: entry: processNode (path ++ [ name ]) entry) node.entries;

      processNode =
        path: node:
        {
          "package" = builtins.addErrorContext "while instantiating package '${lib.showAttrPath path}'" (
            processPackageNode path node
          );
          "package_set" = processPackageSetNode path node;
        }
        .${node.type};
    in
    # Use mapAttrsRecursiveCond to process only package nodes
    {
      packages = processNode [ ] lockedCatalogSpec.packages;
      inherit (lockedCatalogSpec) type;
    };
in
{

  /**
    This function takes a locked `floxhub` catalog
    and instantiates it returning an attribute set of packages
    evaluated from the provided `nixpkgs`.

    `nix` type catalogs (repo based source inputs) are not currently
    supported and cause an evaluation error.

    `floxhub` type catalogs are instantiated by
    1. recurse to find all `type = "package"` entries
    2. instantiate each package by
       2.1. fetching the package source
       2.2. instantiating the package source with `instantiateFromSourceInfo`
       2.3. selecting the package from the instantiated source

    # Example

    ```nix
    let
      nixpkgs = ...;
      floxhubCatalog = {
        packages = {
          hello = {
            source = {
              ref = "refs/heads/main";
              rev = "b59e1a5750b5714c88fb6a7f3232398107704f7b";
              type = "git";
              url = "https://github.com/flox/flox";
            };
            type = "package";
          };
        };
        type = "floxhub";
      };

      # := {type = "floxhub", packages := { hello = <drv> }}
      floxhubCatalogInstance = instantiateCatalog nixpkgs "foo" floxhubCatalog;
    in
      ...

    ```

    # Arguments

    `nixpkgs`
    : an (assumed) nixpkgs instance

    `lockedCatalogSpec`
    : the catalog spec to instantiate
  */
  instantiateCatalog =
    nixpkgs: instantiatedCatalogsClosure: lockedCatalogSpec:
    {
      "nix" = throw "source inputs not currently supported";
      "floxhub" = fetchFloxHubCatalog nixpkgs instantiatedCatalogsClosure lockedCatalogSpec;
    }
    .${lockedCatalogSpec.type};

  /**
    Instantiate multiple catalogs in an attribute set, as provided in a catalog lock file.
    Each attribute is mapped to a catalog instance using `instantiateCatalog`.
  */
  instantiateCatalogs =
    { nixpkgs, catalogSpecClosure }:
    let
      instantiateCatalog' =
        name: catalogSpec:
        builtins.addErrorContext "while instantiating catalog '${name}'" (
          lib.nef.instantiate.instantiateCatalog nixpkgs instantiatedCatalogsClosure catalogSpec
        );
      instantiatedCatalogsClosure = lib.mapAttrs instantiateCatalog' catalogSpecClosure;
    in
    instantiatedCatalogsClosure;

  /**
    Fold every deep override flagged in a catalog closure's lock, plus
    the consuming project's own, into `nixpkgs` as a single overlay
    applied before any catalog or project is instantiated. Overrides
    are discovered under `pkgs/__overrides`, using `lib.nef.dirToAttrs`
    exactly as `pkgs/` itself is discovered; `__overrides` is a
    reserved name, stripped from the ordinary tree by
    `instantiateFromSourceInfo` below. Only sources whose lock entry
    records a deep override are fetched; the rest of the closure is
    left untouched.

    Every override is called as a function against the resulting
    overlay's `final`/`prev` (via `lib.nef.mkOverlay`), never against an
    already-instantiated package, and never against any source's other
    `pkgs/` entries: the overlay built here contains only
    `pkgs/__overrides` entries, and is applied to the base `nixpkgs`
    before `instantiateCatalogs` or `instantiateFromSourceInfo` extend
    it further.

    Two sources overriding the same attribute path is an evaluation
    error naming both (see `mergeOverrideTrees`).

    # Arguments

    `nixpkgs`
    : the base nixpkgs instance deep overrides are applied to

    `catalogSpecClosure`
    : the locked catalog closure, as provided in a catalog lock file

    `sourceInfo`
    : the consuming project's own fetched source
  */
  applyDeepOverrides =
    {
      nixpkgs,
      catalogSpecClosure,
      sourceInfo,
    }:
    let
      overridesTreeOf = label: fetchedSourceInfo: {
        inherit label;
        tree = lib.nef.dirToAttrs "${fetchedSourceInfo.outPath}/${fetchedSourceInfo.dir or ""}/pkgs/${overridesDirName}";
      };

      flaggedSources = lib.unique (collectClosureDeepOverrideSources catalogSpecClosure);
      lockedTrees = map (
        source: overridesTreeOf (labelSource source) (fetchSource source)
      ) flaggedSources;
      ownTree = overridesTreeOf "the consuming project" sourceInfo;

      merged = {
        type = "directory";
        path = "<deep overrides>";
        entries = mergeOverrideTrees [ ] ([ ownTree ] ++ lockedTrees);
      };
    in
    lib.nef.extendAttrSet [ ] { } nixpkgs merged;

  /**
    Instantiate a NEF project from a given sourceInfo.

    * Collects and evaluates packages in `${sourceInfo.outPath}/${sourceInfo.dir or ""}/pkgs`;
    * Exposes the pre-instantiated catalogs supplied via the `instantiatedCatalogsClosure` argument

    Packages are collected and evaluated as an extension of the provided `nixpkgs`,
    using `lib.nef.dirToAttr |> lib.nef.extendAttrSet (nixpkgs // { catalogs = <catalog instances> }).

    In effect all packages have access to the locked catalogs by requesting the `catalogs` attribute.
  */
  instantiateFromSourceInfo =
    {
      nixpkgs,
      sourceInfo,
      instantiatedCatalogsClosure,
    }:

    let
      configRoot = "${sourceInfo.outPath}/${sourceInfo.dir or ""}";
      pkgsDir = configRoot + "/pkgs";

      catalogOverlay = final: prev: {
        catalogs = lib.mapAttrs (
          _: catalogInstance:
          {
            "nix" = catalogInstance.reflect.packages;
            "floxhub" = catalogInstance.packages;
          }
          .${catalogInstance.type}
        ) instantiatedCatalogsClosure;
      };

      nixpkgsWithCatalogs = nixpkgs.extend catalogOverlay;

      # step 1 collect packages
      # `__overrides` (see the reserved-name comment above) lives inside
      # `pkgsDir` but is applied to the shared base by `applyDeepOverrides`,
      # not by this repository's own instantiation; drop it here so it
      # never surfaces as a package set of this repository's own tree.
      collectedPackagesRaw = lib.nef.dirToAttrs pkgsDir;
      collectedPackages = collectedPackagesRaw // {
        entries = builtins.removeAttrs collectedPackagesRaw.entries [ overridesDirName ];
      };

      # Extend nixpkgs, with collectedPackages.
      # `attrPath` and `currentScope` remain empty as this is the toplevel attrset.
      extendedNixpkgs = lib.nef.extendAttrSet [ ] { } nixpkgsWithCatalogs collectedPackages;

      # different forms of identifiers for the collected packages
      # including Make `targets`
      collectedAttrPaths = lib.nef.reflect.collectAttrPaths collectedPackages;

      reflect = {
        attrPaths = collectedAttrPaths;
        targets = lib.nef.reflect.makeTargets collectedAttrPaths;
        packages = lib.nef.reflect.mapToPackages collectedPackages.entries extendedNixpkgs;
      };

    in
    {
      inherit reflect;
      pkgs = extendedNixpkgs;
    };

}
