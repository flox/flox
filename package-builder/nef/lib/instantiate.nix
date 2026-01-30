{ lib }:
{
  nixpkgs,
  pkgsDir,
}:

let

  catalogs =
    let
      catalogLock = "${pkgsDir}/../nix-builds.lock";
      catalogs = if builtins.pathExists catalogLock then (lib.importJSON catalogLock).catalogs else { };
    in
    catalogs;

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
      fetchNixCatalog = builtins.addErrorContext "while fetching catalog '${name}'" (
        let
          lockedRefWithoutDir = builtins.removeAttrs lockedCatalogSpec.locked [ "dir" ];

          tree = builtins.fetchTree lockedRefWithoutDir;
          catalogDotFlox = "${lockedCatalogSpec.locked.dir or ""}/.flox";

          catalogPkgsDir = "${tree.outPath}/${catalogDotFlox}/pkgs";

        in
        lib.nef.instantiate {
          inherit nixpkgs;
          pkgsDir = catalogPkgsDir;
        }
      );

    in
    {
      inherit (lockedCatalogSpec) type;
    }
    // (
      {
        "nix" = fetchNixCatalog;
      }
      .${lockedCatalogSpec.type}
    )
  ) catalogs;

  catalogOverlay = final: prev: {
    catalogs = lib.mapAttrs (
      _: catalogInstance:
      {
        "nix" = catalogInstance.reflect.packages;
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
