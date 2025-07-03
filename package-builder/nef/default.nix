{
  nixpkgs-url ? "nixpkgs",
  nixpkgs-flake ? builtins.getFlake nixpkgs-url,
  pkgs-dir,
  git-subdir ? null,
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
  pkgsDir =
    if git-subdir != null then
      let
        tree = builtins.fetchTree "git+file://${pkgs-dir}";
      in
      "${tree.outPath}/${git-subdir}"

    else
      pkgs-dir;

  lib = nixpkgs.lib.extend libOverlay;
  libOverlay = (import ./lib).overlay;

  # step 1 collect packages
  collectedPackages = lib.nef.dirToAttrs pkgsDir;

  # Extend nixpkgs, with collectedPackages.
  # `attrPath` and `currentScope` remain empty as this is the toplevel attrset.
  extendedNixpkgs = lib.nef.extendAttrSet [ ] { } nixpkgs collectedPackages;

  # different forms of identifiers for the collected packages
  # including Make `targets`
  collectedAttrPaths = lib.nef.reflect.collectAttrPaths collectedPackages;
  reflect = {
    attrPaths = collectedAttrPaths;
    targets = lib.nef.reflect.makeTargets collectedAttrPaths;
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
    assert lib.assertMsg (system != null) ''
      'system' argument missing.
      Evaluate with `--argstr system <SYSTEM>` or with `--impure` to use the current system.
    '';
    extendedNixpkgs;

}
