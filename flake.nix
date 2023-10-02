# ============================================================================ #
#
#
#
# ---------------------------------------------------------------------------- #
{
  inputs.nixpkgs.url = "github:NixOS/nixpkgs";

  # For `gh` CLI ( need a specific version )
  inputs.nixpkgs-for-gh.url = "github:NixOS/nixpkgs/46ed466081b9cad1125b11f11a2af5cc40b942c7";

  # Do not override `nixpkgs` input
  inputs.pkgdb.url = "github:flox/pkgdb";

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
    self,
    nixpkgs,
    nixpkgs-for-gh,
    pkgdb,
    floco,
    parser-util,
    shellHooks,
    crane,
    ...
  } @ inputs: let
    # ------------------------------------------------------------------------ #
    floxVersion = let
      cargoToml =
        builtins.fromTOML (builtins.readFile ./crates/flox/Cargo.toml);
      prefix =
        if self ? revCount
        then "r"
        else "";
      rev = self.revCount or self.shortRev or "dirty";
    in
      cargoToml.package.version + "-" + prefix + (toString rev);

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
      parser-util.overlays.default

      # /Shrinkwrap/ `pkgdb' by cherry picking instead of merging.
      (final: prev: let
        pkgdbPkgsFor = builtins.getAttr prev.system pkgdb.packages;
      in {
        inherit (pkgdbPkgsFor) flox-pkgdb;
      })

      # /Shrinkwrap/ `gh' by cherry picking instead of merging.
      (final: prev: let
        ghPkgsFor =
          builtins.getAttr prev.system nixpkgs-for-gh.legacyPackages;
      in {
        inherit (ghPkgsFor) gh;
      })
    ];

    overlays.flox = final: prev: let
      callPackage = final.lib.callPackageWith (final
        // {
          inherit inputs self floxVersion;
          pkgsFor = final;
          # We need v2.31.0, v2.32.0, or v2.32.1
        });
      genPkg = name: _: callPackage (./pkgs + ("/" + name)) {};
    in
      builtins.mapAttrs genPkg (builtins.readDir ./pkgs);
    overlays.default =
      nixpkgs.lib.composeExtensions overlays.deps
      overlays.flox;

    # ------------------------------------------------------------------------ #

    checks = eachDefaultSystemMap (system: let
      pkgsFor =
        (builtins.getAttr system nixpkgs.legacyPackages).extend
        overlays.default;
      pre-commit-check = pkgsFor.callPackage ./checks/pre-commit-check {
        inherit shellHooks;
        rustfmt = pkgsFor.rustfmt.override {asNightly = true;};
      };
    in {
      inherit pre-commit-check;
      default = pre-commit-check;
    });

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
      checksFor = builtins.getAttr system checks;
      flox = pkgsFor.callPackage ./shells/flox {
        inherit (checksFor) pre-commit-check;
        rustfmt = pkgsFor.rustfmt.override {asNightly = true;};
      };
    in {
      inherit flox;
      default = flox;
      ci = pkgsFor.callPackage ./shells/ci {};
      rust-env = pkgsFor.callPackage ./shells/rust-env {
        inherit (checksFor) pre-commit-check;
        rustfmt = pkgsFor.rustfmt.override {asNightly = true;};
      };
    });
  };
}
# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #

