# ============================================================================ #
#
# A cross-platform environment manager with sharing as a service.
#
# ---------------------------------------------------------------------------- #

{
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/release-23.05";

  inputs.floco.url = "github:aakropotkin/floco";
  inputs.floco.inputs.nixpkgs.follows = "nixpkgs";

  inputs.sqlite3pp.url = "github:aakropotkin/sqlite3pp";
  inputs.sqlite3pp.inputs.nixpkgs.follows = "nixpkgs";

  inputs.parser-util.url = "github:flox/parser-util";
  inputs.parser-util.inputs.nixpkgs.follows = "nixpkgs";

  inputs.shellHooks.url = "github:cachix/pre-commit-hooks.nix";
  inputs.shellHooks.inputs.nixpkgs.follows = "nixpkgs";

  inputs.crane.url = "github:ipetkov/crane";
  inputs.crane.inputs.nixpkgs.follows = "nixpkgs";


# ---------------------------------------------------------------------------- #

  outputs = {
    self,
    nixpkgs,
    floco,
    sqlite3pp,
    parser-util,
    shellHooks,
    crane,
    ...
  } @ inputs: let

# ---------------------------------------------------------------------------- #

    floxVersion = let
      cargoToml = let
        contents = builtins.readFile ./crates/flox/Cargo.toml;
      in builtins.fromTOML contents;
      prefix = if self ? revCount then "r" else "";
      rev    = self.revCount or self.shortRev or "dirty";
    in cargoToml.package.version + "-" + prefix + ( toString rev );


# ---------------------------------------------------------------------------- #

    eachDefaultSystemMap = let
      defaultSystems = [
        "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin"
      ];
    in fn: let
      proc = system: { name = system; value = fn system; };
    in builtins.listToAttrs ( map proc defaultSystems );


# ---------------------------------------------------------------------------- #

    # Add IWYU pragmas
    overlays.nlohmann = final: prev: {
      nlohmann_json = final.callPackage ./pkgs/nlohmann_json.nix {
        inherit (prev) nlohmann_json;
      };
    };

    # Use nix@2.17
    overlays.nix = final: prev: {
      nix = final.callPackage ./pkgs/nix/pkg-fun.nix {};
    };

    # Cherry pick `semver' recipe from `floco'.
    overlays.semver = final: prev: {
      semver = let
        base = final.callPackage "${floco}/fpkgs/semver" {
          nixpkgs = throw ( "`nixpkgs' should not be references when `pkgsFor' "
                            + "is provided"
                          );
          inherit (final) lib;
          pkgsFor = final;
          nodePackage = final.nodejs;
        };
      in base.overrideAttrs ( prevAttrs: { preferLocalBuild = false; } );
    };

    overlays.deps = nixpkgs.lib.composeManyExtensions [
      parser-util.overlays.default  # for `parser-util'
      overlays.nlohmann
      overlays.semver
      overlays.nix
      sqlite3pp.overlays.default
    ];

    overlays.flox = final: prev: let
      callPackage = final.lib.callPackageWith ( final // {
        inherit inputs self floxVersion;
        pkgsFor = final;
      } );
    in {
      flox             = callPackage ./pkgs/flox {};
      flox-bash        = callPackage ./pkgs/flox-bash {};
      flox-bash-dev    = callPackage ./pkgs/flox-bash-dev {};
      flox-dev         = callPackage ./pkgs/flox-dev {};
      flox-env-builder = callPackage ./pkgs/flox-env-builder {};
      flox-gh          = callPackage ./pkgs/flox-gh {};
      flox-src         = callPackage ./pkgs/flox-src {};
      flox-tests       = callPackage ./pkgs/flox-tests {};
      flox-pkgdb       = callPackage ./pkgdb/pkg-fun.nix {};
    };

    overlays.default = nixpkgs.lib.composeExtensions overlays.deps
                                                     overlays.flox;


# ---------------------------------------------------------------------------- #

    # Apply overlays to the `nixpkgs` _base_ set.
    # This is exposed as an output later; but we don't use the name
    # `legacyPackages' to avoid checking the full closure with
    # `nix flake check' and `nix search'.
    pkgsFor = eachDefaultSystemMap ( system: let
      base = builtins.getAttr system nixpkgs.legacyPackages;
    in base.extend overlays.default );


# ---------------------------------------------------------------------------- #

    checks = eachDefaultSystemMap ( system: let
      pkgs = builtins.getAttr system pkgsFor;
    in {
      pre-commit-check = pkgs.callPackage ./checks/pre-commit-check {
        inherit shellHooks;
        rustfmt = pkgs.rustfmt.override {asNightly = true;};
      };
    } );


# ---------------------------------------------------------------------------- #

    packages = eachDefaultSystemMap ( system: let
      pkgs = builtins.getAttr system pkgsFor;
    in {
      inherit
        (pkgs)
        flox
        flox-pkgdb
        flox-env-builder
        flox-bash
        flox-gh
        flox-tests
        ;
      default            = pkgs.flox;
      flox-tests-end2end = pkgs.flox-tests.override {
        testsDir = "/tests/end2end";
      };
    } );


# ---------------------------------------------------------------------------- #

  in {

    inherit overlays packages pkgsFor checks;

    devShells = eachDefaultSystemMap (system: let
      pkgs      = builtins.getAttr system pkgsFor;
      checksFor = builtins.getAttr system checks;
      flox = pkgs.callPackage ./shells/flox {
        inherit (checksFor) pre-commit-check;
        rustfmt = pkgs.rustfmt.override { asNightly = true; };
      };
    in {
      inherit flox;
      default  = flox;
      ci       = pkgs.callPackage ./shells/ci {};
      pkgdb    = pkgs.callPackage ./shells/pkgdb/pkg-fun.nix { ci = false; };
      pkgdb-ci = pkgs.callPackage ./shells/pkgdb/pkg-fun.nix { ci = true; };
    } );

  };  # End `outputs'


# ---------------------------------------------------------------------------- #

}


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
