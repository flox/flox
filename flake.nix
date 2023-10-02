# ============================================================================ #
#
#
#
# ---------------------------------------------------------------------------- #
{
  inputs.nixpkgs.url = "github:NixOS/nixpkgs";

  inputs.pkgdb.url = "github:flox/pkgdb";
  inputs.pkgdb.inputs.nixpkgs.follows = "/nixpkgs";

  inputs.floco.follows = "/pkgdb/floco";
  inputs.floco.inputs.nixpkgs.follows = "/nixpkgs";

  inputs.parser-util.url = "github:flox/parser-util/v0";
  inputs.parser-util.inputs.nixpkgs.follows = "/nixpkgs";

  inputs.shellHooks.url = "github:cachix/pre-commit-hooks.nix";
  inputs.shellHooks.inputs.nixpkgs.follows = "/nixpkgs";

  inputs.crane.url = "github:ipetkov/crane";
  inputs.crane.inputs.nixpkgs.follows = "/nixpkgs";
  inputs.crane.inputs.flake-compat.follows = "/shellHooks/flake-compat";
  inputs.crane.inputs.flake-utils.follows = "/shellHooks/flake-utils";

  # -------------------------------------------------------------------------- #

  outputs = {
    nixpkgs,
    pkgdb,
    floco,
    parser-util,
    shellHooks,
    crane,
    ...
  } @ inputs: let
    # ------------------------------------------------------------------------ #
    eachDefaultSystemMap = let
      defaultSystems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
    in
      fn: let
        proc = system: {
          name = system;
          value = fn system;
        };
      in
        builtins.listToAttrs (map proc defaultSystems);

    # ------------------------------------------------------------------------ #

    overlays.deps = nixpkgs.lib.composeManyExtensions [
      pkgdb.overlays.default
      parser-util.overlays.default
    ];
    overlays.flox = final: prev: let
      callPackage = final.lib.callPackageWith (final
        // {
          inherit inputs;
          self = toString ./.;
        });
      genPkg = name: _: callPackage (./pkgs + ("/" + name)) {};
    in
      builtins.mapAttrs genPkg (builtins.readDir ./pkgs);
    overlays.default =
      nixpkgs.lib.composeExtensions overlays.deps
      overlays.flox;

    # ------------------------------------------------------------------------ #

    packages = eachDefaultSystemMap (system: let
      pkgsFor =
        (builtins.getAttr system nixpkgs.legacyPackages).extend
        overlays.default;
    in {
      inherit
        (pkgsFor)
        flox
        builtfilter-rs
        flox-bash
        flox-gh
        flox-src
        flox-tests
        nix-editor
        ;
      default = pkgsFor.flox;
    });
    # ------------------------------------------------------------------------ #
  in {
    inherit overlays packages;

    devShells = eachDefaultSystemMap (system: let
      pkgsFor =
        (builtins.getAttr system nixpkgs.legacyPackages).extend
        overlays.default;
      flox = pkgsFor.callPackage ./shells/flox {};
    in {
      inherit flox;
      default = flox;
      ci = pkgsFor.callPackage ./shells/ci {};
      rust-env = pkgsFor.callPackage ./shells/rust-env {};
    });
  };
}
# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #

