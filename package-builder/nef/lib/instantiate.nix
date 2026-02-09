{ lib }:
{
  nixpkgs,
  pkgsDir,
  catalogsLock ? null,
}:

let
  catalogs =
    if catalogsLock != null && builtins.pathExists catalogsLock then
      (lib.importJSON catalogsLock).catalogs
    else
      { };

  catalogInstances = lib.mapAttrs (
    name: lockedCatalogSpec:

    # "catalogs": {
    #   "foo": {
    #     "hash": "sha256-/UmRJVt7XpE27LGxS2hgGKWsErTx1oe65jhwWNPsnYs=",
    #     "pkgsDir": ".flox/pkgs",
    #     "catalogsLock": ".flox/nix-builds.lock",
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
          sourceInfo = builtins.fetchTree lockedCatalogSpec.locked;
          root = sourceInfo.outPath;
        in
        builtins.addErrorContext "while fetching catalog '${name}'" (
          lib.nef.instantiate {
            inherit nixpkgs;
            pkgsDir = "${root}/${lockedCatalogSpec.pkgsDir}";
            catalogsLock =
              if lockedCatalogSpec ? catalogsLock then "${root}/${lockedCatalogSpec.catalogsLock}" else null;
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
