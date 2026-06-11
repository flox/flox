{
  nixpkgs-url ? "nixpkgs",
  nixpkgs-flake ? builtins.getFlake nixpkgs-url,
  source-ref,
  catalog-lockfile ? throw "A catalog lockfile is required to evaluate packages",
  system ? builtins.currentSystem or null,
}:
let
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

      let
        sourceInfo = builtins.fetchTree (builtins.removeAttrs parsedRef [ "dir" ]);
      in
      sourceInfo // lib.optionalAttrs (parsedRef ? dir) { inherit (parsedRef) dir; };

  catalogSpecClosure = (lib.importJSON catalog-lockfile).catalogs;
  instantiatedCatalogsClosure = lib.nef.instantiate.instantiateCatalogs {
    inherit nixpkgs catalogSpecClosure;
  };

in
lib.nef.instantiate.instantiateFromSourceInfo {
  inherit nixpkgs instantiatedCatalogsClosure sourceInfo;
}
