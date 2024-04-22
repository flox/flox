# ============================================================================ #
#
# A cross-platform environment manager with sharing as a service.
#
# ---------------------------------------------------------------------------- #
{
  description = "flox - Harness the power of Nix";

  nixConfig.extra-substituters = [ "https://cache.flox.dev" ];
  nixConfig.extra-trusted-public-keys = [
    "flox-cache-public-1:7F4OyH7ZCnFhcze3fJdfyXYLQw/aV7GEed86nQ7IsOs="
  ];

  # XXX Temporary: lock process-compose to v1.9 until we can update flox to use
  # the latest version. v1.9 did not appear on any stable snapshots so we instead
  # select the most recent staging branch commit on which it appeared.
  inputs.nixpkgs-process-compose.url = "github:flox/nixpkgs/staging.20240817";
  inputs.nixpkgs-process-compose.flake = false;

  # Roll forward monthly as **our** stable branch advances. Note that we also
  # build against the staging branch in CI to detect regressions before they
  # reach stable.
  inputs.nixpkgs.url = "github:flox/nixpkgs/stable";

  inputs.sqlite3pp.url = "github:aakropotkin/sqlite3pp";
  inputs.sqlite3pp.inputs.nixpkgs.follows = "nixpkgs";

  inputs.pre-commit-hooks.url = "github:cachix/pre-commit-hooks.nix";
  inputs.pre-commit-hooks.inputs.nixpkgs.follows = "nixpkgs";

  inputs.crane.url = "github:ipetkov/crane";

  inputs.fenix.url = "github:nix-community/fenix";
  inputs.fenix.inputs.nixpkgs.follows = "nixpkgs";

  # -------------------------------------------------------------------------- #

  outputs =
    inputs:
    let
      # ------------------------------------------------------------------------ #
      # Temporarily use nixpkgs-process-compose
      nixpkgs.legacyPackages = {
        inherit (inputs.nixpkgs.legacyPackages)
          x86_64-linux
          x86_64-darwin
          aarch64-linux
          aarch64-darwin
          ;
      };
      nixpkgs.lib = inputs.nixpkgs.lib;
    in
    rec {
      # Overlays
      # --------
      overlays.deps = nixpkgs.lib.composeManyExtensions [
        (final: prev: {
          process-compose = final.callPackage (
            inputs.nixpkgs-process-compose + "/pkgs/applications/misc/process-compose"
          ) { };

          # Add IWYU pragmas to `nlohmann_json'
          # ( _include what you use_ extensions to headers for static analysis )
          nlohmann_json = final.callPackage ./pkgs/nlohmann_json { inherit (prev) nlohmann_json; };

          # Uncomment to compile Nix with debug symbols on Linux
          # nix = final.enableDebugging (final.callPackage ./pkgs/nix {});
          nix = final.callPackage ./pkgs/nix { };

          cpp-semver = final.callPackage ./pkgs/cpp-semver { };
        })
        inputs.sqlite3pp.overlays.default
        inputs.fenix.overlays.default
      ];

      # Packages defined in this repository.
      overlays.flox =
        final: prev:
        let
          callPackage = final.lib.callPackageWith (
            final
            // {
              inherit inputs; # passing in inputs... beware
              inherit (inputs) self;
              pkgsFor = final;
            }
          );
        in
        {
          # Generates a `.git/hooks/pre-commit' script.
          pre-commit-check = callPackage ./pkgs/pre-commit-check { inherit (inputs) pre-commit-hooks; };

          GENERATED_DATA = ./test_data/generated;
          MANUALLY_GENERATED = ./test_data/manually_generated;

          # We depend on several nightly features of rustfmt,
          # so pick the current nightly version.
          # We're using `default.withComponents`
          # which _should_ only pull the nightly rustfmt component.
          # Alternatively, we could use nixpkgs.rustfmt,
          # and rebuild with a (stable) fenix toolchain and `asNightly = true`,
          # which would avoid the need to pull another channel altogether.
          rustfmt = final.fenix.default.withComponents [ "rustfmt" ];
          rust-toolchain = final.fenix.stable;

          rust-external-deps = callPackage ./pkgs/rust-external-deps { };
          rust-internal-deps = callPackage ./pkgs/rust-internal-deps { };

          # (Linux-only) LD_AUDIT library for using dynamic libraries in Flox envs.
          ld-floxlib = callPackage ./pkgs/ld-floxlib { };
          flox-src = callPackage ./pkgs/flox-src { };
          flox-activation-scripts = callPackage ./pkgs/flox-activation-scripts { };
          flox-package-builder = callPackage ./pkgs/flox-package-builder { };

          # Package Database Utilities: scrape, search, and resolve.
          flox-pkgdb = callPackage ./pkgs/flox-pkgdb { };
          flox-buildenv = callPackage ./pkgs/flox-buildenv {
            nixpkgsClone = inputs.nixpkgs.outPath;
          };
          flox-watchdog = callPackage ./pkgs/flox-watchdog { }; # Flox Command Line Interface ( development build ).
          flox-activations = callPackage ./pkgs/flox-activations { };
          flox-cli = callPackage ./pkgs/flox-cli { };
          flox-manpages = callPackage ./pkgs/flox-manpages { }; # Flox Command Line Interface Manpages
          flox = callPackage ./pkgs/flox { }; # Flox Command Line Interface ( production build ).

          # Wrapper scripts for running test suites.
          flox-cli-tests = callPackage ./pkgs/flox-cli-tests { };
        };

      # Composes dependency overlays and the overlay defined here.
      overlays.default = nixpkgs.lib.composeExtensions overlays.deps overlays.flox;

      # ------------------------------------------------------------------------ #

      # Apply overlays to the `nixpkgs` _base_ set.
      # This is exposed as an output later; but we don't use the name
      # `legacyPackages' to avoid checking the full closure with
      # `nix flake check' and `nix search'.
      pkgsContext = builtins.mapAttrs (system: pkgs: pkgs.extend overlays.default) nixpkgs.legacyPackages;

      # ------------------------------------------------------------------------ #

      checks = builtins.mapAttrs (system: pkgs: { inherit (pkgs) pre-commit-check; }) pkgsContext;

      # ------------------------------------------------------------------------ #

      packages = builtins.mapAttrs (system: pkgs: {
        inherit (pkgs)
          flox-activation-scripts
          flox-pkgdb
          flox-buildenv
          flox-package-builder
          flox-watchdog
          flox-activations
          flox-cli
          flox-cli-tests
          flox-manpages
          flox
          ld-floxlib
          pre-commit-check
          rust-external-deps
          rust-internal-deps
          ;
        default = pkgs.flox;
      }) pkgsContext;

      # ------------------------------------------------------------------------ #
      devShells = builtins.mapAttrs (
        system: pkgsBase:
        let
          pkgs = pkgsBase.extend (
            final: prev: {
              flox-cli-tests = prev.flox-cli-tests.override {
                PROJECT_TESTS_DIR = "/cli/tests";
                PKGDB_BIN = null;
                FLOX_BIN = null;
                WATCHDOG_BIN = null;
              };
              flox-cli = prev.flox-cli.override {
                flox-pkgdb = null;
                flox-watchdog = null;
              };
              checksFor = checks.${final.system};
            }
          );
        in
        {
          default = pkgs.callPackage ./shells/default { };
        }
      ) pkgsContext;
    }; # End `outputs'

  # -------------------------------------------------------------------------- #
}
