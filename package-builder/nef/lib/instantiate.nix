{ lib }:
{
  nixpkgs,
  sourceInfo,
}:

let
  configRoot = "${sourceInfo.outPath}/${sourceInfo.dir or ""}";

  pkgsDir = configRoot + "/pkgs";
  catalogsLock = configRoot + "/nix-builds.lock";

  catalogs = if builtins.pathExists catalogsLock then (lib.importJSON catalogsLock).catalogs else { };

  catalogInstances = lib.mapAttrs (
    name: lockedCatalogSpec:

    # "catalogs": {
    #   "foo": {
    #     "hash": "sha256-/UmRJVt7XpE27LGxS2hgGKWsErTx1oe65jhwWNPsnYs=",
    #     "locked": {
    #       "lastModified": 1769623709,
    #       "ref": "refs/heads/main",
    #       "rev": "b59e1a5750b5714c88fb6a7f3232398107704f7b",
    #       "revCount": 4504,
    #       "type": "git",
    #       "url": "https://github.com/flox/flox"
    #     },
    #     "original": {
    #       "type": "git",
    #       "url": "https://github.com/flox/flox"
    #     },
    #     "storePath": "/nix/store/df0qd3gnkix513br8az06yrnspg28530-source"
    #   },
    #   [...]
    # }
    let
      fetchNixCatalog =
        let
          sourceInfo =
            let
              lockedWithoutDir = builtins.removeAttrs lockedCatalogSpec.locked [ "dir" ];
              sourceInfo = builtins.fetchTree lockedWithoutDir;
            in
            sourceInfo
            // lib.optionalAttrs (lockedCatalogSpec.locked ? dir) { inherit (lockedCatalogSpec.locked) dir; };
        in
        builtins.addErrorContext "while fetching catalog '${name}'" (
          lib.nef.instantiate {
            inherit nixpkgs sourceInfo;
          }
        );

      fetchFloxHubCatalog =
        let

          # Function to process a package node
          processPackageNode =
            path: lockedPackageSpec:
            let
              byBuildType = {
                "nef" =
                  let
                    sourceInfo =
                      let
                        lockedWithoutDir = builtins.removeAttrs lockedPackageSpec.source [ "dir" ];
                        sourceInfo = builtins.fetchTree lockedWithoutDir;
                      in
                      sourceInfo
                      // lib.optionalAttrs (lockedPackageSpec.source ? dir) { inherit (lockedPackageSpec.source) dir; };

                    instantiatedCatalog = lib.nef.instantiate {
                      inherit nixpkgs sourceInfo;
                    };
                    instantiatedPackage = lib.getAttrFromPath path instantiatedCatalog.reflect.packages;
                  in
                  # Return the instantiated environment
                  # The catalog overlay will use .reflect.packages
                  instantiatedPackage;
                "manifest" = throw "Manifest build type not supported in Nix expressions";
              };
            in
            byBuildType.${lockedPackageSpec.build_type};
          processPackageSetNode =
            path: node: lib.mapAttrs (name: entry: matchNode (path ++ [ name ]) entry) node.entries;
          matchNode =
            path: node:
            {
              "package" = processPackageNode path node;
              "package_set" = processPackageSetNode path node;
            }
            .${node.type};
        in
        # Use mapAttrsRecursiveCond to process only package nodes
        {
          packages = matchNode [ ] lockedCatalogSpec.packages;
        };
    in
    {
      inherit (lockedCatalogSpec) type;
    }
    // (
      {
        "nix" = fetchNixCatalog;
        "floxhub" = fetchFloxHubCatalog;
      }
      .${lockedCatalogSpec.type}
    )
  ) catalogs;

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
}
