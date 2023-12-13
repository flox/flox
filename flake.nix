# ============================================================================ #
#
# A cross-platform environment manager with sharing as a service.
#
# ---------------------------------------------------------------------------- #
{
  description = "flox - Harness the power of Nix";

  nixConfig.extra-substituters = [
    "https://cache.flox.dev"
  ];
  nixConfig.extra-trusted-public-keys = [
    "flox-cache-public-1:7F4OyH7ZCnFhcze3fJdfyXYLQw/aV7GEed86nQ7IsOs="
  ];

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/release-23.11";

  # drop once bear is no longer broken in a newer release
  inputs.nixpkgs-bear.url = "github:NixOS/nixpkgs/release-23.05";

  inputs.floco.url = "github:aakropotkin/floco";
  inputs.floco.inputs.nixpkgs.follows = "nixpkgs";

  inputs.sqlite3pp.url = "github:aakropotkin/sqlite3pp";
  inputs.sqlite3pp.inputs.nixpkgs.follows = "nixpkgs";

  inputs.pre-commit-hooks.url = "github:cachix/pre-commit-hooks.nix";
  inputs.pre-commit-hooks.inputs.nixpkgs.follows = "nixpkgs";

  inputs.crane.url = "github:ipetkov/crane";
  inputs.crane.inputs.nixpkgs.follows = "nixpkgs";

  # This is needed to be able to calculate `git describe` format version of flox
  # without running `git describe`
  inputs.flox-latest.url = "git+ssh://git@github.com/flox/flox?ref=latest";
  inputs.flox-latest.flake = false;

  # -------------------------------------------------------------------------- #

  outputs = {
    self,
    nixpkgs,
    floco,
    sqlite3pp,
    pre-commit-hooks,
    crane,
    flox-latest,
    ...
  } @ inputs: let
    # Given a function `fn' which takes system names as an argument, produce an
    # attribute set whose keys are system names, and values are the result of
    # applying that system name to `fn'.
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

    # Overlays
    # --------

    # Add IWYU pragmas to `nlohmann_json'
    # ( _include what you use_ extensions to headers for static analysis )
    overlays.nlohmann = final: prev: {
      nlohmann_json = final.callPackage ./pkgs/nlohmann_json {
        inherit (prev) nlohmann_json;
      };
    };

    # Use nix@2.17
    overlays.nix = final: prev: {
      nix = final.callPackage ./pkgs/nix {};
    };

    # Cherry pick `semver' recipe from `floco'.
    overlays.semver = final: prev: {
      cpp-semver = final.callPackage ./pkgs/cpp-semver {};
    };

    # bear is broken in release 23.11 on darwin
    overlays.bear = final: prev: {
      inherit (inputs.nixpkgs-bear.legacyPackages.${prev.system}) bear;
    };

    # Aggregates all external dependency overlays before adding any of the
    # packages defined by this flake.
    overlays.deps = nixpkgs.lib.composeManyExtensions [
      overlays.nlohmann
      overlays.semver
      overlays.nix
      overlays.bear
      sqlite3pp.overlays.default
    ];

    # Packages defined in this repository.
    overlays.flox = final: prev: let
      callPackage = final.lib.callPackageWith (final
        // {
          inherit inputs self;
          pkgsFor = final;
        });
    in {
      # Use bleeding edge `rustfmt'.
      rustfmt = prev.rustfmt.override {asNightly = true;};

      # Generates a `.git/hooks/pre-commit' script.
      pre-commit-check = pre-commit-hooks.lib.${final.system}.run {
        src = builtins.path {path = ./.;};
        default_stages = ["manual" "push"];
        hooks = {
          alejandra.enable = true;
          clang-format = {
            enable = true;
            types_or = final.lib.mkForce [
              "c"
              "c++"
            ];
          };
          rustfmt = let
            wrapper = final.symlinkJoin {
              name = "rustfmt-wrapped";
              paths = [final.rustfmt];
              nativeBuildInputs = [final.makeWrapper];
              postBuild = let
                PATH = final.lib.makeBinPath [final.cargo final.rustfmt];
              in ''
                wrapProgram $out/bin/cargo-fmt --prefix PATH : ${PATH};
              '';
            };
          in {
            enable = true;
            entry = final.lib.mkForce "${wrapper}/bin/cargo-fmt fmt --all --manifest-path 'cli/Cargo.toml' -- --color always";
          };
          clippy.enable = true;
          commitizen.enable = true;
          shfmt.enable = false;
          # shellcheck.enable = true; # disabled until we have time to fix all the warnings
        };
        settings = {
          alejandra.verbosity = "quiet";
          rust.cargoManifestPath = "cli/Cargo.toml";
        };
        tools = {
          clang-tools = final.clang-tools_16;
        };
      };

      # Customized `gh' executable used for auth.
      flox-gh = callPackage ./pkgs/flox-gh {};

      # Package Database Utilities: scrape, search, and resolve.
      flox-pkgdb = callPackage ./pkgs/flox-pkgdb {};

      # Flox Command Line Interface ( development build ).
      flox-cli = callPackage ./pkgs/flox-cli {};

      # Flox Command Line Interface Manpages
      flox-manpages = callPackage ./pkgs/flox-manpages {};

      # Flox Command Line Interface ( production build ).
      flox = callPackage ./pkgs/flox {};

      # Wrapper scripts for running test suites.
      flox-cli-tests = callPackage ./pkgs/flox-cli-tests {};
      # Integration tests
      flox-tests = callPackage ./pkgs/flox-tests {};
      flox-tests-pure = callPackage ./pkgs/flox-tests-pure {inputs = inputs;};
    };

    # Composes dependency overlays and the overlay defined here.
    overlays.default =
      nixpkgs.lib.composeExtensions overlays.deps
      overlays.flox;

    # ------------------------------------------------------------------------ #

    # Apply overlays to the `nixpkgs` _base_ set.
    # This is exposed as an output later; but we don't use the name
    # `legacyPackages' to avoid checking the full closure with
    # `nix flake check' and `nix search'.
    pkgsFor = eachDefaultSystemMap (system: let
      base = builtins.getAttr system nixpkgs.legacyPackages;
    in
      base.extend overlays.default);

    # ------------------------------------------------------------------------ #

    checks = eachDefaultSystemMap (system: let
      pkgs = builtins.getAttr system pkgsFor;
    in {
      inherit (pkgs) pre-commit-check;
    });

    # ------------------------------------------------------------------------ #

    packages = eachDefaultSystemMap (system: let
      pkgs = builtins.getAttr system pkgsFor;
    in {
      inherit
        (pkgs)
        flox-gh
        flox-pkgdb
        flox-cli
        flox-cli-tests
        flox-manpages
        flox
        pre-commit-check
        flox-tests-pure
        flox-dev
        ;
      default = pkgs.flox;
    });
    # ------------------------------------------------------------------------ #
  in {
    inherit overlays packages pkgsFor checks;

    devShells = eachDefaultSystemMap (system: let
      pkgsBase = builtins.getAttr system pkgsFor;
      pkgs = pkgsBase.extend (final: prev: {
        flox-cli-tests = prev.flox-cli-tests.override {
          PROJECT_TESTS_DIR = "/cli/tests";
          PKGDB_BIN = null;
          FLOX_BIN = null;
        };
        flox-tests = prev.flox-tests.override {
          PROJECT_TESTS_DIR = "/tests";
          PKGDB_BIN = null;
          FLOX_BIN = null;
        };
        flox-cli = prev.flox-cli.override {flox-pkgdb = null;};
      });
      checksFor = builtins.getAttr system checks;
    in {
      default = pkgs.callPackage ./shells/default {
        inherit (checksFor) pre-commit-check;
      };
    });
  }; # End `outputs'

  # -------------------------------------------------------------------------- #
}
