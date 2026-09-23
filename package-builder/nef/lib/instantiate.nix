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

  # Walk a `dirToAttrs`-shaped tree (see ./dirToAttrs.nix) down `remaining`,
  # erroring against the reference point `fullAttrPath` and `sourceLabel`
  # when a component is missing: the source a global override was locked
  # against no longer has the entry its attr_path names.
  getOverrideNode =
    sourceLabel: fullAttrPath: tree: remaining:
    if remaining == [ ] then
      tree
    else
      let
        name = builtins.head remaining;
        child =
          tree.entries.${name} or (throw (
            "Global override '${lib.showAttrPath fullAttrPath}' from '${sourceLabel}' "
            + "was locked, but its source's 'pkgs/' tree has no '${name}' entry."
          ));
      in
      getOverrideNode sourceLabel fullAttrPath child (builtins.tail remaining);

  # Any one sourceLabel reachable under a merged-tree directory node --
  # used to name "the other side" of a prefix collision (see
  # insertOverrideNode below), where the conflicting entry isn't a single
  # leaf but a whole subtree of one or more overrides underneath it.
  anyLabelUnder =
    node:
    if node.type or null == "directory" then
      anyLabelUnder (lib.head (builtins.attrValues node.entries))
    else
      node.sourceLabel;

  # Insert one locked override's node into a merged, `dirToAttrs`-shaped
  # extensions tree at `attrPath`, tagging it with `sourceLabel` so a
  # later insert at the same path -- or at a path this one is a prefix or
  # an extension of -- can name both sources. Two overrides claiming the
  # same attribute are ambiguous: nixpkgs would only ever see one of
  # them, whether they name the exact same path (`openssl` twice) or one
  # names a path the other is nested under (`foo` and `foo.bar` can't
  # both exist -- `foo` can't be both a package and a directory).
  #
  # The collision itself is never thrown here: this only assembles the
  # overlay's tree shape, ahead of any package being built, so the throw
  # is stored as the colliding leaf's `path` instead of being returned
  # directly. `mkOverlay`'s "nix" case only reads a node's `path` when
  # that specific attribute is actually forced (`callPackage value.path`),
  # the same way its own `recursionGuardError` defers its throw to
  # `prev.${name}` rather than raising it while the overlay is built.
  insertOverrideNode =
    sourceLabel: node: attrPath:
    let
      go =
        prefixSoFar: remaining: tree:
        let
          name = builtins.head remaining;
          rest = builtins.tail remaining;
          conflictPath = prefixSoFar ++ [ name ];
          existing = tree.entries.${name} or null;
          isDirectory = existing != null && existing.type or null == "directory";
          collisionWith =
            otherLabel:
            throw (
              "Two global overrides target '${lib.showAttrPath conflictPath}': "
              + "'${otherLabel}' and '${sourceLabel}'."
            );
          setEntry =
            value:
            tree
            // {
              entries = tree.entries // {
                ${name} = value;
              };
            };
        in
        if rest == [ ] then
          setEntry (
            if existing == null then
              node // { inherit sourceLabel; }
            else if isDirectory then
              # `existing` is an interior node a longer attr_path put here
              # (this entry is e.g. 'foo', the other is 'foo.bar' or
              # deeper) -- every leaf under it shares this prefix, so any
              # one names the other side.
              node
              // {
                inherit sourceLabel;
                path = collisionWith (anyLabelUnder existing);
              }
            else
              node
              // {
                inherit sourceLabel;
                path = collisionWith existing.sourceLabel;
              }
          )
        else if existing != null && !isDirectory then
          # `existing` is a leaf another override already claimed exactly
          # at this prefix (this entry is e.g. 'foo.bar', the other is
          # 'foo') -- there is nothing under a leaf to descend into, so
          # poison it in place and stop.
          setEntry (existing // { path = collisionWith existing.sourceLabel; })
        else
          let
            childTree =
              if existing == null then
                {
                  type = "directory";
                  path = null;
                  entries = { };
                }
              else
                existing;
          in
          setEntry (go conflictPath rest childTree);
    in
    go [ ] attrPath;

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
      collectedPackages = lib.nef.dirToAttrs pkgsDir;

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

  /**
    Build the single overlay that applies every locked global override to
    the shared base nixpkgs, ahead of `instantiateCatalogs`.

    A global override is a published package, so it is defined in its
    source's `pkgs/` tree alongside every ordinary package that source
    publishes -- unlike the per-package deep-override approaches, there
    is no separate `overrides/` tree to draw a structural line around.
    Isolation is instead a check: for each entry, this discovers the
    source's `pkgs/` tree with `lib.nef.dirToAttrs` and extracts only
    the single node at the entry's locked `attr_path`, ignoring every
    other entry the tree contains. Every extracted node is merged into
    one extensions tree before a single call to `lib.nef.mkOverlay`, so
    an override runs once, against `final`, with neither its own
    repository's other `pkgs/` entries nor `catalogs` in scope: both
    are added afterwards, by `instantiateFromSourceInfo` and
    `instantiateCatalogs`, against the nixpkgs this overlay already
    extended. `catalogs` is bound to a throw in the scope passed to
    `mkOverlay` (see its `currentScope` argument), rather than simply
    left out, since `nixpkgs.extend` re-folds this overlay into
    whatever `final` a later `.extend` produces -- including one that
    already carries `catalogs` -- and an absent binding would resolve
    to that leaked value instead of failing.

    # Arguments

    `overridesClosure`
    : the `global_overrides` group of a catalog lock -- a map from lock
      key to `{ attr_path, source, ... }`, the same `LockedInputEntry`
      shape `direct_catalog_inputs` uses.
  */
  mkGlobalOverridesOverlay =
    { overridesClosure }:
    let
      entries = lib.mapAttrsToList (
        key: entry:
        let
          # The lock key alone identifies the source in error messages --
          # it is already the lock's own name for this entry, and unlike
          # `entry.source` it does not assume a git-shaped flakeref.
          sourceLabel = key;
          sourceInfo = builtins.addErrorContext "while fetching global override source '${sourceLabel}'" (
            fetchSource entry.source
          );
          pkgsRoot = "${sourceInfo.outPath}/${sourceInfo.dir or ""}/pkgs";
          pkgsTree = lib.nef.dirToAttrs pkgsRoot;
          node =
            builtins.addErrorContext
              "while resolving global override '${lib.showAttrPath entry.attr_path}' from '${sourceLabel}'"
              (getOverrideNode sourceLabel entry.attr_path pkgsTree entry.attr_path);
        in
        {
          inherit sourceLabel node;
          attrPath = entry.attr_path;
        }
      ) overridesClosure;

      merged = builtins.foldl' (tree: e: insertOverrideNode e.sourceLabel e.node e.attrPath tree) {
        type = "directory";
        path = null;
        entries = { };
      } entries;

      # Bound in the scope every override is called with (see
      # `lib.nef.mkOverlay`'s `currentScope` argument), so referencing
      # `catalogs` fails only for an override that actually asks for
      # it, not for every override applied here.
      catalogsDeniedError = throw ''
        A global override cannot use catalog packages: it is folded
        into the base nixpkgs before any catalog is instantiated, so
        no catalog exists yet when it runs.
      '';
    in
    lib.nef.mkOverlay [ ] { catalogs = catalogsDeniedError; } merged;

  /**
    Apply an overlay assembled by `mkGlobalOverridesOverlay` to
    `nixpkgs`, extending it exactly as `instantiateFromSourceInfo`
    above extends `nixpkgs` for a project's own `pkgs/`.

    # Arguments

    `nixpkgs`
    : the base nixpkgs instance the overlay is applied to

    `overlay`
    : the overlay returned by `mkGlobalOverridesOverlay`
  */
  applyGlobalOverridesOverlay = nixpkgs: overlay: nixpkgs.extend overlay;

}
