{
  treefmt,
  nixfmt-rfc-style,
  commitizen,
  hivemind,
  just,
  yq,
  lib,
  mkShell,
  procps,
  pre-commit-check,
  pstree,
  shfmt,
  mitmproxy,
  cargo-nextest,
  flox-cli,
  flox-cli-tests,
  flox-activations,
  flox-watchdog,
  flox-pkgdb,
  stdenv,
  ci ? false,
}:
let
  # For use in GitHub Actions and local development.
  ciPackages = [ ] ++ flox-pkgdb.ciPackages;

  devPackages =
    flox-pkgdb.devPackages
    ++ flox-cli.devPackages
    ++ [
      just
      hivemind
      commitizen
      treefmt
      nixfmt-rfc-style
      shfmt
      yq
      cargo-nextest
      procps
      pstree
      flox-cli-tests
    ]
    ++ lib.optionals stdenv.isLinux [
      # The python3Packages.mitmproxy-macos package is broken on mac:
      #   nix-repl> legacyPackages.aarch64-darwin.python3Packages.mitmproxy-macos.meta.broken
      #   true
      # ... so only install it on Linux. It's only an optional dev dependency.
      mitmproxy
    ];
in
mkShell (
  {
    name = "flox-dev";

    # Artifacts not build by nix, i.e. cargo builds
    # generally all cargo builds should have the same inputs
    # but in case we add specific ones,
    # it's good to have them here already.
    inputsFrom = [
      flox-pkgdb
      flox-cli
      flox-watchdog
      flox-activations
    ];

    packages = ciPackages ++ lib.optionals (!ci) devPackages;

    shellHook =
      pre-commit-check.shellHook
      + ''
        function define_dev_env_var() {
          local USAGE="Usage: define_dev_env_var <name> <value>";

          local name=''${1?$USAGE};
          local value=''${2?$USAGE};

          export $name="$value";
          echo "$name => $(printenv "$name")";
        }

        # Find the project root.
        REPO_ROOT="$( git rev-parse --show-toplevel; )";

        # Setup mutable paths to all internal subsystems,
        # so that they can be changed and built without restarting the shell.

        # cargo built binaries
        define_dev_env_var FLOX_BIN "''${REPO_ROOT}/cli/target/debug/flox";
        define_dev_env_var WATCHDOG_BIN "''${REPO_ROOT}/cli/target/debug/flox-watchdog";
        define_dev_env_var FLOX_ACTIVATIONS_BIN "''${REPO_ROOT}/cli/target/debug/flox-activations";

        # make built binaries
        define_dev_env_var PKGDB_BIN "''${REPO_ROOT}/pkgdb/bin/pkgdb";

        # static nix files
        define_dev_env_var FLOX_MK_CONTAINER_NIX "''${REPO_ROOT}/mkContainer/mkContainer.nix";

        # Nix built subsystems
        define_dev_env_var FLOX_INTERPRETER "''${REPO_ROOT}/build/flox-activation-scripts";
        define_dev_env_var FLOX_BUILDENV "''${REPO_ROOT}/build/flox-buildenv";
        define_dev_env_var FLOX_BUILDENV_NIX "''${FLOX_BUILDENV}/lib/buildenv.nix";
        define_dev_env_var FLOX_PACKAGE_BUILDER "''${REPO_ROOT}/build/flox-package-builder";
        define_dev_env_var FLOX_BUILD_MK "''$FLOX_PACKAGE_BUILDER/libexec/flox-build.mk";
        define_dev_env_var FLOX_MANPAGES "''${REPO_ROOT}/build/flox-manpages";

        # test data
        define_dev_env_var GENERATED_DATA "''${REPO_ROOT}/test_data/generated";
        define_dev_env_var MANUALLY_GENERATED "''${REPO_ROOT}/test_data/manually_generated";

        # Add all internal rust crates to the path.
        # That's `flox` itself as well as the `flox-watchdog`
        # and `flox-activations` subsystems.
        export PATH="''${REPO_ROOT}/cli/target/debug":$PATH;

        # Add the pkgdb binary to the path
        export PATH="''${REPO_ROOT}/pkgdb/bin":$PATH;

        # Add the flox-manpages to the manpath
        export MANPATH="''${FLOX_MANPAGES}/share/man:$MANPATH"

        echo;
        echo "run 'just build' to build flox and all its subsystems";
      '';
  }
  // flox-pkgdb.devEnvs
  // flox-watchdog.devEnvs
  // flox-cli.devEnvs
)
