{
  nixpkgs-url ? "nixpkgs",
  nixpkgs-flake ? builtins.getFlake nixpkgs-url,
  source-ref,
  catalog-lockfile ? throw "A catalog lockfile is required to evaluate packages",
  system ? builtins.currentSystem or null,
}:
let
  # The NEF overlay exposes this same pinned instance as `catalogs.nixpkgs`.
  nixpkgs = import nixpkgs-flake {
    inherit system;
    config = {
      allowUnfree = true;
      allowInsecure = true;
    };
  };
  libOverlay = (import ./lib).overlay;
  lib = nixpkgs-flake.lib.extend libOverlay;

  parsedRef =
    if builtins.isAttrs source-ref then
      source-ref
    else if builtins.isString source-ref then
      builtins.parseFlakeRef source-ref
    else
      throw "'source-ref' needs to be a flakeref url or structure, was ${builtins.typeOf source-ref}";

  sourceInfo =
    if parsedRef.type == "path" then
      { outPath = parsedRef.path; } // lib.optionalAttrs (parsedRef ? dir) { inherit (parsedRef) dir; }
    else
      lib.nef.instantiate.fetchSource parsedRef;

  catalogSpecClosure = (lib.importJSON catalog-lockfile).catalogs;

  # Every package in the build must see the same base nixpkgs, so deep
  # overrides are folded in before any catalog is instantiated.
  deepOverrides = lib.nef.instantiate.collectDeepOverrides {
    inherit catalogSpecClosure sourceInfo;
  };
  nixpkgsWithDeepOverrides = lib.nef.instantiate.applyDeepOverrides nixpkgs deepOverrides;

  instantiatedCatalogsClosure = lib.nef.instantiate.instantiateCatalogs {
    nixpkgs = nixpkgsWithDeepOverrides;
    inherit catalogSpecClosure;
  };

in
lib.nef.instantiate.instantiateFromSourceInfo {
  nixpkgs = nixpkgsWithDeepOverrides;
  inherit instantiatedCatalogsClosure sourceInfo;
}
