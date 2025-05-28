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

  # different forms of identifiers for the collected packages
  # including Make `targets`
  collectedAttrPaths = lib.nef.reflect.collectAttrPaths [ ] collectedPackages;
  reflect = {
    attrPaths = lib.nef.reflect.attrPathStrings collectedAttrPaths;
    targets = lib.nef.reflect.makeTargets collectedAttrPaths extendedNixpkgs;
  };

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

  # get make targets
  #
  # nix eval -f <nef> --argstr pkgs-dir <PATH> reflect.targets
  # nix eval -f <nef> --argstr pkgs-dir <PATH> reflect.attrPaths
  inherit reflect;

  # get all the packages
  #
  # nix eval -f <nef> --argstr pkgs-dir <PATH> --argstr system <SYSTEM> pkgs.<attrPath>
  pkgs =
    assert lib.assertMsg (system != null)
      "system missing, needs top be provided with `--argstr system <SYSTEM>` or by running with `--impure`";
    extendedNixpkgs;

}
