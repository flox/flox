# ============================================================================ #
#
# A cross-platform environment manager with sharing as a service.
#
# ---------------------------------------------------------------------------- #
{
  description = "flox - Harness the power of Nix";

  nixConfig.extra-substituters = [
    "https://cache.floxdev.com"
  ];

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/release-23.05";

  inputs.floco.url = "github:aakropotkin/floco";
  inputs.floco.inputs.nixpkgs.follows = "nixpkgs";

  inputs.sqlite3pp.url = "github:aakropotkin/sqlite3pp";
  inputs.sqlite3pp.inputs.nixpkgs.follows = "nixpkgs";

  inputs.parser-util.url = "github:flox/parser-util";
  inputs.parser-util.inputs.nixpkgs.follows = "nixpkgs";

  inputs.pre-commit-hooks.url = "github:cachix/pre-commit-hooks.nix";
  inputs.pre-commit-hooks.inputs.nixpkgs.follows = "nixpkgs";

  inputs.crane.url = "github:ipetkov/crane";
  inputs.crane.inputs.nixpkgs.follows = "nixpkgs";

  # ---------------------------------------------------------------------------- #

  outputs = {
    self,
    nixpkgs,
    floco,
    sqlite3pp,
    parser-util,
    pre-commit-hooks,
    crane,
    ...
  } @ inputs: let
    # ---------------------------------------------------------------------------- #
    floxVersion = let
      cargoToml = let
        contents = builtins.readFile ./cli/flox/Cargo.toml;
      in
        builtins.fromTOML contents;
      prefix =
        if self ? revCount
        then "r"
        else "";
      rev = self.revCount or self.shortRev or "dirty";
    in
      cargoToml.package.version + "-" + prefix + (toString rev);

    # ---------------------------------------------------------------------------- #

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

    # ---------------------------------------------------------------------------- #

    # Add IWYU pragmas
    overlays.nlohmann = final: prev: {
      nlohmann_json = final.callPackage ./pkgs/nlohmann_json.nix {
        inherit (prev) nlohmann_json;
      };
    };

    # Use nix@2.17
    overlays.nix = final: prev: {
      nix = final.callPackage ./pkgs/nix {};
    };

    # Cherry pick `semver' recipe from `floco'.
    overlays.semver = final: prev: {
      semver = let
        base = final.callPackage "${floco}/fpkgs/semver" {
          nixpkgs = throw (
            "`nixpkgs' should not be references when `pkgsFor' "
            + "is provided"
          );
          inherit (final) lib;
          pkgsFor = final;
          nodePackage = final.nodejs;
        };
      in
        base.overrideAttrs (prevAttrs: {preferLocalBuild = false;});
    };

    overlays.deps = nixpkgs.lib.composeManyExtensions [
      parser-util.overlays.default # for `parser-util'
      overlays.nlohmann
      overlays.semver
      overlays.nix
      sqlite3pp.overlays.default
    ];

    overlays.flox = final: prev: let
      callPackage = final.lib.callPackageWith (final
        // {
          inherit inputs self floxVersion;
          pkgsFor = final;
        });
    in {
      rustfmt = prev.rustfmt.override {asNightly = true;};
      pre-commit-check = pre-commit-hooks.lib.${final.system}.run {
        src = builtins.path {path = ./.;};
        hooks = {
          alejandra.enable = true;
          rustfmt2 = let
            wrapper = final.symlinkJoin {
              name = "rustfmt-wrapped";
              paths = [final.rustfmt];
              nativeBuildInputs = [final.makeWrapper];
              postBuild = ''
                wrapProgram $out/bin/cargo-fmt \
                  --prefix PATH : ${final.lib.makeBinPath [final.cargo final.rustfmt]}
              '';
            };
          in {
            enable = true;
            name = "rustfmt";
            description = "Format Rust code.";
            entry = "${wrapper}/bin/cargo-fmt fmt --all --manifest-path 'cli/Cargo.toml' -- --color always";
            files = "\\.rs$";
            pass_filenames = false;
          };
          commitizen.enable = true;
        };
      };

      flox-gh = callPackage ./pkgs/flox-gh {};

      flox-pkgdb = callPackage ./pkgs/flox-pkgdb {};
      flox-env-builder = callPackage ./pkgs/flox-env-builder {};
      flox-cli = callPackage ./pkgs/flox-cli {};

      flox = callPackage ./pkgs/flox-cli {longVersion = true;};

      flox-pkgdb-tests = callPackage ./pkgs/flox-pkgdb-tests {};
      flox-env-builder-tests = callPackage ./pkgs/flox-env-builder-tests {};
      flox-cli-tests = callPackage ./pkgs/flox-cli-tests {};

      flox-tests = callPackage ./pkgs/flox-tests {};
    };

    overlays.default =
      nixpkgs.lib.composeExtensions overlays.deps
      overlays.flox;

    # ---------------------------------------------------------------------------- #

    # Apply overlays to the `nixpkgs` _base_ set.
    # This is exposed as an output later; but we don't use the name
    # `legacyPackages' to avoid checking the full closure with
    # `nix flake check' and `nix search'.
    pkgsFor = eachDefaultSystemMap (system: let
      base = builtins.getAttr system nixpkgs.legacyPackages;
    in
      base.extend overlays.default);

    # ---------------------------------------------------------------------------- #

    checks = eachDefaultSystemMap (system: let
      pkgs = builtins.getAttr system pkgsFor;
    in {
      inherit (pkgs) pre-commit-check;
    });

    # ---------------------------------------------------------------------------- #

    packages = eachDefaultSystemMap (system: let
      pkgs = builtins.getAttr system pkgsFor;
    in {
      inherit
        (pkgs)
        flox-gh
        flox-pkgdb
        flox-env-builder
        flox-cli
        flox
        pre-commit-check
        ;
      default = pkgs.flox;
    });
    # ---------------------------------------------------------------------------- #
  in {
    inherit overlays packages pkgsFor checks;

    devShells = eachDefaultSystemMap (system: let
      pkgs = builtins.getAttr system pkgsFor;
      checksFor = builtins.getAttr system checks;
    in {
      default = pkgs.callPackage ./shells/default {
        inherit (checksFor) pre-commit-check;
      };
    });
  }; # End `outputs'

  # ---------------------------------------------------------------------------- #
}
# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #

