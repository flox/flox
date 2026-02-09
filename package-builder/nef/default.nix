{
  nixpkgs-url ? "nixpkgs",
  nixpkgs-flake ? builtins.getFlake nixpkgs-url,
  source-ref,
  pkgs-dir ? ".",
  catalogs-lock ? null,
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

  parsedRef =
    if builtins.isAttrs source-ref then
      source-ref
    else if builtins.isString source-ref then
      builtins.parseFlakeRef source-ref
    else
      throw "'source-ref' needs to be a flakeref url or structure, was ${builtins.typeOf source-ref}";

  sourceInfo = builtins.fetchTree parsedRef;
  root = sourceInfo.outPath;

  pkgsDir = "${root}/${pkgs-dir}";
  catalogsLock = if catalogs-lock != null then "${root}/${catalogs-lock}" else null;

  libOverlay = (import ./lib).overlay;
  lib = nixpkgs.lib.extend libOverlay;
in
lib.nef.instantiate {
  inherit nixpkgs pkgsDir catalogsLock;
}
