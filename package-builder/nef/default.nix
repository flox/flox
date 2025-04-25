{
  nixpkgs-url ? "nixpkgs",
  nixpkgs-flake ? builtins.getFlake nixpkgs-url,
  pkgs-dir,
  system ? builtins.currentSystem or null,
}:
let
  nixpkgs = import nixpkgs-flake {
    inherit system; # todo set config
  };
  pkgsDir = pkgs-dir;

  lib = nixpkgs.lib.extend libOverlay;
  libOverlay = (import ./lib).overlay;

  # step 1 collect packages
  collectedPackages = lib.nef.dirToAttrs pkgsDir;

  # Extend nixpkgs, with collectedPackages.
  # `attrPath` and `currentScope` remain empty as this is the toplevel attrset.
  extendedNixpkgs = lib.nef.extendAttrSet [ ] { } nixpkgs collectedPackages;
in
{
  # debugging stuff ignore for now
  inherit
    lib
    libOverlay
    nixpkgs
    collectedPackages
    extendedNixpkgs
    ;

  # get all the packages
  #
  # nix eval -f <nef> --argstr pkgs-dir <PATH> --argstr system <SYSTEM> pkgs.<attrPath>
  pkgs =
    assert lib.assertMsg (system != null)
      "system missing, needs top be provided with `--argstr system <SYSTEM>` or by running with `--impure`";
    extendedNixpkgs;

}
