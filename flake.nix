# ============================================================================ #
#
#
#
# ---------------------------------------------------------------------------- #
{
  description = "flox - Harness the power of Nix";

  nixConfig.extra-substituters = [
    "https://cache.floxdev.com"
    "s3://flox-store"
  ];

  # Do not override `nixpkgs` input
  inputs.pkgdb.url = "github:flox/pkgdb";

  inputs.nixpkgs.follows = "pkgdb/nixpkgs";

  inputs.floco.follows = "pkgdb/floco";
  inputs.floco.inputs.nixpkgs.follows = "nixpkgs";

  inputs.parser-util.url = "github:flox/parser-util";
  inputs.parser-util.inputs.nixpkgs.follows = "nixpkgs";

  inputs.shellHooks.url = "github:cachix/pre-commit-hooks.nix";
  inputs.shellHooks.inputs.nixpkgs.follows = "nixpkgs";

  inputs.crane.url = "github:ipetkov/crane";
  inputs.crane.inputs.nixpkgs.follows = "nixpkgs";

  # -------------------------------------------------------------------------- #

  outputs = {
    self,
    nixpkgs,
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

    # /Shrinkwrap/ `pkgdb' to preserve `cc' and `nix' versions.
    overlays.pkgdb-shrinkwrap = final: prev: let
      pkgdbPkgsFor = builtins.getAttr prev.system pkgdb.packages;
    in {
      inherit (pkgdbPkgsFor) flox-pkgdb;
    };

    overlays.deps = nixpkgs.lib.composeManyExtensions [
      parser-util.overlays.default # for `parser-util'
      floco.overlays.default # for `semver'
      pkgdb.overlays.default
      overlays.pkgdb-shrinkwrap
    ];

    overlays.flox = final: prev: let
      callPackage = final.lib.callPackageWith (final
        // {
          inherit inputs self floxVersion;
          pkgsFor = final;
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
    in {
      pre-commit-check = pkgsFor.callPackage ./checks/pre-commit-check {
        inherit shellHooks;
        rustfmt = pkgsFor.rustfmt.override {asNightly = true;};
      };
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
        flox-env-builder
        flox-bash
        flox-gh
        flox-tests
        ;
      default = pkgsFor.flox;
      flox-tests-ci = pkgsFor.flox-tests.override {
        FLOX_CLI = "${pkgsFor.flox}/bin/flox";
      };
      flox-tests-end2end = pkgsFor.flox-tests.override {
        testsDir = "/tests/end2end";
      };
      flox-tests-end2end-ci = pkgsFor.flox-tests.override {
        testsDir = "/tests/end2end";
        FLOX_CLI = "${pkgsFor.flox}/bin/flox";
      };
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
    });
  };
}
# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #

