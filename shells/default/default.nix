{
  treefmt,
  nixfmt-rfc-style,
  commitizen,
  hivemind,
  just,
  yq,
  lib,
  mkShell,
  writeShellScript,
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
  stdenv,
  ci ? false,
}:
let
  # For use in GitHub Actions and local development.

  devPackages =
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

  envWrapper = writeShellScript "wrapper" ''
    BUILD_DIR="$( cd "$( dirname "''${BASH_SOURCE[0]}" )" &> /dev/null && pwd )";
    ENV_CMD="/usr/bin/env -";

    # Load the envs from the .env file
    for env in "$(cat $BUILD_DIR/.env)"; do
      ENV_CMD="$ENV_CMD $env";
    done

    # Prepend the PATH from the .PATH file
    ENV_CMD="$ENV_CMD PATH=$(cat $BUILD_DIR/.PATH):$PATH";

    # Run the command with the environment
    ENV_CMD="$ENV_CMD";

    exec $ENV_CMD "$@";
  '';

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
        # Find the project root.
        REPO_ROOT="$( git rev-parse --show-toplevel; )";

        mkdir -p "$REPO_ROOT/build";
        rm -f "$REPO_ROOT/build/.env"; # clear the .env file
        rm -f "$REPO_ROOT/build/.PATH"; # clear the .PATH file
        cp -f ${envWrapper} "$REPO_ROOT/build/wrapper";


        # Define a function to set an environment variable
        # and add it to the .env file.
        function define_dev_env_var() {
          local USAGE="Usage: define_dev_env_var <name> <value>";

          local name=''${1?$USAGE};
          local value=''${2?$USAGE};

          export $name="$value";
          echo "$name=$value" >> "$REPO_ROOT/build/.env";

          echo "$name => $(printenv "$name")";

        }

        # Setup mutable paths to all internal subsystems,
        # so that they can be changed and built without restarting the shell.

        # cargo built binaries
        define_dev_env_var FLOX_BIN "''${REPO_ROOT}/cli/target/debug/flox";
        define_dev_env_var WATCHDOG_BIN "''${REPO_ROOT}/cli/target/debug/flox-watchdog";
        define_dev_env_var FLOX_ACTIVATIONS_BIN "''${REPO_ROOT}/cli/target/debug/flox-activations";

        # make built binaries

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

        # Add all internal rust crates to the PATH.
        # That's `flox` itself as well as the `flox-watchdog`
        # and `flox-activations` subsystems.
        export PATH="''${REPO_ROOT}/cli/target/debug":$PATH;
        echo -n "''${REPO_ROOT}/cli/target/debug:" >> "$REPO_ROOT/build/.PATH";

        # Add the flox-manpages to the manpath
        export MANPATH="''${FLOX_MANPAGES}/share/man:$MANPATH"

        echo;
        echo "run 'just build' to build flox and all its subsystems";
      '';
  }
  // flox-watchdog.devEnvs
  // flox-cli.devEnvs
)
