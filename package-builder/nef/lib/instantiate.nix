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

  # {
  #   hash = "sha256-/UmRJVt7XpE27LGxS2hgGKWsErTx1oe65jhwWNPsnYs=";
  #   locked = {
  #     lastModified = 1769623709;
  #     ref = "refs/heads/main";
  #     rev = "b59e1a5750b5714c88fb6a7f3232398107704f7b";
  #     revCount = 4504;
  #     type = "git";
  #     url = "https://github.com/flox/flox";
  #   };
  #   original = {
  #     type = "git";
  #     url = "https://github.com/flox/flox";
  #   };
  #   storePath = "/nix/store/df0qd3gnkix513br8az06yrnspg28530-source";
  #   type = "nix";
  # }
  fetchNixCatalog =
    nixpkgs: lockedCatalogSpec:
    let
      sourceInfo = fetchSource lockedCatalogSpec.locked;
    in
    lib.nef.instantiate.instantiateFromSourceInfo {
      inherit nixpkgs sourceInfo;

    }
    // {
      inherit (lockedCatalogSpec) type;
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
    nixpkgs: lockedCatalogSpec:
    let
      # process a package node
      processPackageNode =
        path: lockedPackageSpec:
        {
          "nef" =
            let
              sourceInfo = fetchSource lockedPackageSpec.source;
              instantiatedCatalog = lib.nef.instantiate.instantiateFromSourceInfo {
                inherit nixpkgs sourceInfo;
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
    This function takes a locked catalog
    (either a repo based `nix` catalog or floxhub catalog)
    and instantiates it returning an attribute set of packages
    evaluated from the provided `nixpkgs`.

    `nix` type catalogs are instantiated by
    1. fetching their locked source,
    2. instantiating the source (recursively) with `instantiateFromSourceInfo`

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
      nixCatalog = {
        locked = {
          lastModified = 1769623709;
          ref = "refs/heads/main";
          rev = "b59e1a5750b5714c88fb6a7f3232398107704f7b";
          revCount = 4504;
          type = "git";
          url = "https://github.com/flox/flox";
        };
        original = { ... };
        type = "nix";
      };
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

      # := {type = "nix", reflect := { packages, ...}, ... }
      nixCatalogInstance = instantiateCatalog nixpkgs "foo" nixCatalog;
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
    nixpkgs: lockedCatalogSpec:
    {
      "nix" = fetchNixCatalog nixpkgs lockedCatalogSpec;
      "floxhub" = fetchFloxHubCatalog nixpkgs lockedCatalogSpec;
    }
    .${lockedCatalogSpec.type};

  /**
    Instantiate multiple catalogs in an attribute set, as provided in a catalog lock file.
    Each attribute is mapped to a catalog instance using `instantiateCatalog`.
  */
  instantiateCatalogs =
    { nixpkgs, catalogs }:
    let
      instantiateCatalog' =
        name: catalogSpec:
        builtins.addErrorContext "while instantiating catalog '${name}'" (
          lib.nef.instantiate.instantiateCatalog nixpkgs catalogSpec
        );
    in
    lib.mapAttrs instantiateCatalog' catalogs;

  /**
    Instantiate a NEF project from a given sourceInfo.

    * Collects and evaluates packages in `${sourceInfo.outPath}/${sourceInfo.dir or ""}/pkgs`;
    * Instantiates locked catalogs defined in `${sourceInfo.outPath}/${sourceInfo.dir or ""}/nix-builds.lock`

    Packages are collected and evaluated as an extension of the provided `nixpkgs`,
    using `lib.nef.dirToAttr |> lib.nef.extendAttrSet (nixpkgs // { catalogs = <catalog instances> }).

    In effect all packages have access to the locked catalogs by requesting the `catalogs` attribute.
  */
  instantiateFromSourceInfo =
    {
      nixpkgs,
      sourceInfo,
    }:

    let
      configRoot = "${sourceInfo.outPath}/${sourceInfo.dir or ""}";

      pkgsDir = configRoot + "/pkgs";
      catalogsLock = configRoot + "/nix-builds.lock";

      catalogs =
        if builtins.pathExists catalogsLock then
          let
            lockfile = (lib.importJSON catalogsLock);
          in
          # We have _internally_ published a few builds without a lockfile version.
          # TODO: require a version before GA?
          if !(lockfile ? version) || lockfile.version == 1 then
            lockfile.catalogs
          else
            builtins.throw "unsupported catalog lockfile version"
        else
          builtins.throw ''
            `nix-builds.lock` not found, run `flox build update-catalogs` to generate it
            and add it to version control.
          '';

      catalogInstances = lib.nef.instantiate.instantiateCatalogs {
        inherit nixpkgs catalogs;
      };

      catalogOverlay = final: prev: {
        catalogs = lib.mapAttrs (
          _: catalogInstance:
          {
            "nix" = catalogInstance.reflect.packages;
            "floxhub" = catalogInstance.packages;
          }
          .${catalogInstance.type}
        ) catalogInstances;
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
      # debug
      inherit catalogInstances catalogs;

      #/debug
      inherit reflect;
      pkgs = extendedNixpkgs;
    };

}
