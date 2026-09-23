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

  # A human-readable label for a locked source, for collision and
  # provenance messages. Mirrors the rendering `fetchSource` already
  # uses for its own error context.
  describeSource = source: builtins.flakeRefToString (builtins.removeAttrs source [ "dir" ]);

  # Every locked package source in a floxhub package tree (as read from
  # the catalog lockfile), recursing through package_set nodes to each
  # package leaf's `source`.
  collectPackageSources =
    node:
    {
      "package" = [ node.source ];
      "package_set" = lib.concatMap collectPackageSources (lib.attrValues node.entries);
    }
    .${node.type};

  # Every locked package source across the whole catalog closure.
  collectClosureSources =
    catalogSpecClosure:
    lib.concatMap (catalogSpec: collectPackageSources catalogSpec.packages) (
      lib.attrValues catalogSpecClosure
    );

  # Deep overrides live under this name inside a repository's `pkgs/`
  # tree (e.g. `pkgs/__overrides/openssl`), so an override's attr path
  # is publishable like any other package. `instantiateFromSourceInfo`
  # excludes this name so it never surfaces as an ordinary package set.
  deepOverridesDirName = "__overrides";

  # The `pkgs/__overrides/` directory of a fetched source, as a
  # `dirToAttrs` tree, or `null` if the source has none. The target
  # attribute path is the path below `__overrides`, so
  # `pkgs/__overrides/python3Packages/foo` targets `python3Packages.foo`.
  overridesTreeOf =
    sourceInfo:
    let
      overridesDir = "${sourceInfo.outPath}/${sourceInfo.dir or ""}/pkgs/${deepOverridesDirName}";
    in
    if builtins.pathExists overridesDir then lib.nef.dirToAttrs overridesDir else null;

  # Union the `pkgs/__overrides/` trees of every source into one tree
  # of the same shape `dirToAttrs` returns for a single directory.
  #
  # An override is a package definition shadowing a base attribute,
  # with no identifier of its own, so there is nothing to prefer one
  # over another by. Two sources defining the same attribute path is
  # therefore an evaluation error naming both, rather than one
  # silently shadowing the other.
  mergeOverrideTrees =
    attrPath: labeledTrees:
    let
      names = lib.unique (lib.concatMap (t: lib.attrNames t.tree.entries) labeledTrees);
      mergeName =
        name:
        let
          childAttrPath = attrPath ++ [ name ];
          matches = map (t: {
            inherit (t) label;
            node = t.tree.entries.${name};
          }) (lib.filter (t: t.tree.entries ? ${name}) labeledTrees);
        in
        lib.nameValuePair name (
          if lib.length matches == 1 then
            (lib.head matches).node
          else if lib.all (m: m.node.type == "directory") matches then
            mergeOverrideTrees childAttrPath (
              map (m: {
                inherit (m) label;
                tree = m.node;
              }) matches
            )
          else
            throw ''
              Deep override collision at '${lib.showAttrPath childAttrPath}':
              defined by both ${lib.concatMapStringsSep " and " (m: m.label) matches}.
            ''
        );
    in
    {
      type = "directory";
      path = null;
      entries = lib.listToAttrs (map mergeName names);
    };

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
    Union the `pkgs/__overrides/` directories of every locked package
    source in `catalogSpecClosure`, plus the consuming project's own,
    into one override tree, of the same shape `dirToAttrs` returns for
    a single directory.

    Every override is discovered rather than declared in the lock, so
    every locked source in the closure is fetched here, whether or not
    the consumer references a package from it.

    Returns `null` when no source in the closure carries a
    `pkgs/__overrides/` directory, so `applyDeepOverrides` can tell
    "nothing to apply" apart from an empty tree.

    # Arguments

    `catalogSpecClosure`
    : the full locked catalog closure, as read from the catalog lockfile

    `selfSourceInfo`
    : the consuming project's own fetched source info, whose
      `pkgs/__overrides/` directory (if any) is unioned in alongside
      its dependencies'
  */
  collectDeepOverrides =
    {
      catalogSpecClosure,
      selfSourceInfo,
    }:
    let
      dependencyLabeledSources = map (source: {
        label = describeSource source;
        sourceInfo = fetchSource source;
      }) (collectClosureSources catalogSpecClosure);

      labeledSources = dependencyLabeledSources ++ [
        {
          label = "the project itself";
          sourceInfo = selfSourceInfo;
        }
      ];

      labeledTrees = lib.filter (t: t.tree != null) (
        map (s: {
          inherit (s) label;
          tree = overridesTreeOf s.sourceInfo;
        }) labeledSources
      );
    in
    if labeledTrees == [ ] then null else mergeOverrideTrees [ ] labeledTrees;

  /**
    Apply an override tree assembled by `collectDeepOverrides` to
    `nixpkgs`.

    Reuses `extendAttrSet` exactly as `instantiateFromSourceInfo` does
    for a project's `pkgs/`, so an override is called as a function
    against the evolving `final`, never applied as an already-
    instantiated package. Because this runs before any catalog or
    `pkgs/` tree is instantiated, an override can only see the base and
    other overrides — never its own repository's `pkgs/`.

    # Arguments

    `nixpkgs`
    : the base nixpkgs to apply deep overrides to

    `overrideTree`
    : the tree returned by `collectDeepOverrides`, or `null` for "no
      overrides found", in which case `nixpkgs` is returned unchanged
  */
  applyDeepOverrides =
    nixpkgs: overrideTree:
    if overrideTree == null then nixpkgs else lib.nef.extendAttrSet [ ] { } nixpkgs overrideTree;

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
      #
      # `pkgs/__overrides/` is applied to the base nixpkgs separately,
      # before this repository's own `pkgs/` is collected (see
      # `applyDeepOverrides`), and excluded here so it never surfaces
      # as a package set a consumer of this repository could reference.
      collectedPackages =
        let
          rawCollectedPackages = lib.nef.dirToAttrs pkgsDir;
        in
        rawCollectedPackages
        // {
          entries = builtins.removeAttrs rawCollectedPackages.entries [ deepOverridesDirName ];
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
