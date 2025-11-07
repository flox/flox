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

  # Roll forward monthly as **our** stable branch advances. Note that we also
  # build against the staging branch in CI to detect regressions before they
  # reach stable.
  inputs.nixpkgs.url = "github:flox/nixpkgs/stable";

  inputs.pre-commit-hooks.url = "github:cachix/pre-commit-hooks.nix";
  inputs.pre-commit-hooks.inputs.nixpkgs.follows = "nixpkgs";

  inputs.crane.url = "github:ipetkov/crane";

  inputs.fenix.url = "github:nix-community/fenix";
  inputs.fenix.inputs.nixpkgs.follows = "nixpkgs";

  # Include upstram nix-unit (with added flake support)
  # until a new release is tagged and available in nixpkgs.
  # Avoid management of 'nixpkgs' and other flake inputs
  # since we will add nix-unit via an overlay to make use of our nix patches.
  inputs.nix-unit.url = "github:nix-community/nix-unit";
  inputs.nix-unit.inputs.nixpkgs.follows = "nixpkgs";

  # -------------------------------------------------------------------------- #

  outputs =
    inputs:
    let
      # ------------------------------------------------------------------------ #
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
          # Add IWYU pragmas to `nlohmann_json'
          # ( _include what you use_ extensions to headers for static analysis )
          nlohmann_json = final.callPackage ./pkgs/nlohmann_json { inherit (prev) nlohmann_json; };

          # Uncomment to compile Nix with debug symbols on Linux
          # nix = final.enableDebugging (final.callPackage ./pkgs/nix {});
          nix = final.callPackage ./pkgs/nix { };

          cpp-semver = final.callPackage ./pkgs/cpp-semver { };

          process-compose = final.callPackage ./pkgs/process-compose { inherit (prev) process-compose; };

        })
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
          nixpkgsInputLockedURL =
            nixpkgsInput: "github:flox/nixpkgs/${nixpkgsInput.rev}?narHash=${nixpkgsInput.narHash}";
        in
        {
          # Generates a `.git/hooks/pre-commit' script.
          pre-commit-check = callPackage ./pkgs/pre-commit-check { inherit (inputs) pre-commit-hooks; };

          GENERATED_DATA = ./test_data/generated;
          UNIT_TEST_GENERATED = ./test_data/unit_test_generated;
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
          rust-internal-deps = callPackage ./pkgs/rust-internal-deps { inherit nixpkgsInputLockedURL; };

          # (Linux-only) LD_AUDIT library for using dynamic libraries in Flox envs.
          ld-floxlib = callPackage ./pkgs/ld-floxlib { };
          flox-src = callPackage ./pkgs/flox-src { };
          flox-interpreter = callPackage ./pkgs/flox-interpreter { };
          flox-package-builder = callPackage ./pkgs/flox-package-builder { };

          # Package Database Utilities: scrape, search, and resolve.
          flox-nix-plugins = callPackage ./pkgs/flox-nix-plugins { };
          flox-buildenv = callPackage ./pkgs/flox-buildenv { };
          flox-watchdog = callPackage ./pkgs/flox-watchdog { }; # Flox Command Line Interface ( development build ).
          flox-activations = callPackage ./pkgs/flox-activations { };
          flox-cli = callPackage ./pkgs/flox-cli { };
          flox-manpages = callPackage ./pkgs/flox-manpages { }; # Flox Command Line Interface Manpages
          flox = callPackage ./pkgs/flox { }; # Flox Command Line Interface ( production build ).

          # Wrapper scripts for running test suites.
          flox-cli-tests = callPackage ./pkgs/flox-cli-tests { };
        };

      overlays.development = final: prev: {
        floxDevelopmentPackages =
          let
            # Create a flox-activations package that just copies the Cargo built
            # development binary into $out/libexec/flox-activations
            floxActivationsBin = "${builtins.path { path = builtins.getEnv "FLOX_ACTIVATIONS_BIN"; }}";
            cargoBuiltFloxActivations =
              prev.runCommandNoCC "flox-activations"
                {
                  name = "flox-activations";
                  path = floxActivationsBin;
                }
                ''
                  mkdir -p $out/libexec
                  ln -s ${floxActivationsBin} $out/libexec/flox-activations
                '';
          in
          prev.lib.makeScope prev.newScope (self: {
            rust-internal-deps = prev.rust-internal-deps.override {
              flox-buildenv = null;
              flox-package-builder = null;
              flox-nix-plugins = null;
              flox-mk-container = null;
            };

            flox-cli = prev.flox-cli.override {
              flox-interpreter = null;
              flox-watchdog = null;
              rust-internal-deps = self.rust-internal-deps;
            };
            flox-watchdog = prev.flox-watchdog.override {
              rust-internal-deps = self.rust-internal-deps;
            };
            flox-activations = prev.flox-activations.override { };
            flox-interpreter = prev.flox-interpreter.override {
              flox-activations = cargoBuiltFloxActivations;
            };
            flox-package-builder = prev.flox-package-builder.override { };
            flox-buildenv = prev.flox-buildenv.override {
              flox-interpreter = null;
              flox-activations = cargoBuiltFloxActivations;
            };
            checksFor = checks.${prev.system};

            flox-cli-tests = prev.flox-cli-tests.override {
              PROJECT_TESTS_DIR = "/cli/tests";
              localDev = true;
            };
            # TODO: we would prefer using nix-unit from nixpkgs, but it hasn't been updated.
            # We can't currently use an overlay because nix-unit doesn't support
            # as late of a version of Nix as we're using.
            nix-unit = inputs.nix-unit.packages.${prev.system}.nix-unit;
          });
      };
      # Composes dependency overlays and the overlay defined here.
      overlays.default = nixpkgs.lib.composeManyExtensions [
        overlays.deps
        overlays.flox
        overlays.development
      ];

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
          flox-interpreter
          flox-nix-plugins
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
          floxDevelopmentPackages
          nix
          ;

        default = pkgs.flox;
      }) pkgsContext;

      # ------------------------------------------------------------------------ #

      devShells = builtins.mapAttrs (system: pkgsBase: {
        default = pkgsBase.floxDevelopmentPackages.callPackage ./shells/default { };
      }) pkgsContext;

      # ------------------------------------------------------------------------ #

      # NixOS/Darwin/HomeManager module
      nixosModules.flox = import ./modules/nixos.nix pkgsContext;
      darwinModules.flox = import ./modules/darwin.nix pkgsContext;
      homeModules.flox = import ./modules/home.nix pkgsContext;

      # ------------------------------------------------------------------------ #
    };

  # -------------------------------------------------------------------------- #
}
