# Dev shell whose build inputs come from the crate2nix graph instead of the
# crane packages, for comparison against `shells/default` (CLI-164). One of the
# two goes away once the migration lands.
#
# The difference is `inputsFrom`: crane builds the workspace in one derivation
# that carries openssl, krb5, pkg-config and bindgen directly, while
# buildRustCrate attaches those to the crate that needs them. mkShell reads
# only the direct inputs of what it is given, so this shell is pointed at every
# crate in the graph rather than at the three binaries. Nothing beyond tooling
# is listed by hand.
{
  cargo-nextest,
  commitizen,
  crate2nix,
  crate2nix-builds,
  daemonize,
  flox-cli-tests,
  flox-nix-plugins,
  hivemind,
  jq,
  just,
  lib,
  mkShell,
  nix-unit,
  nixfmt-rfc-style,
  podman,
  pre-commit-check,
  procps,
  pstree,
  rust-toolchain,
  rustfmt ? rust-toolchain.rustfmt,
  shfmt,
  system,
  treefmt,
  writeShellScript,
  yamlfmt,
  yq,
  ci ? false,
}:
let
  # Byte-stable fixed-output derivation that publish unit tests use as a real
  # store path. Realised as part of the dev shell so the tests can rely on it
  # being present instead of building it themselves.
  fixedTestStorePath = import ../../test_data/manually_generated/cli-128-fixed-empty.nix {
    inherit system;
  };

  ciPackages = flox-nix-plugins.ciPackages;

  devPackages = flox-nix-plugins.devPackages ++ [
    cargo-nextest
    commitizen
    crate2nix
    daemonize
    flox-cli-tests
    hivemind
    jq
    just
    nix-unit
    nixfmt-rfc-style
    podman
    procps
    pstree
    rustfmt
    shfmt
    treefmt
    yamlfmt
    yq
  ];

  # cargo builds the crates in this shell, so it needs what buildRustCrate
  # would have given each of them: openssl from openssl-sys, krb5 and bindgen
  # from libgssapi-sys, the toolchain from every crate. Taking the closure of
  # each workspace member rather than of `flox` alone is what reaches the
  # build-dependency subtrees, `catalog-api-v1`'s progenitor among them.
  crateInputs =
    let
      crates = lib.concatMap (
        member: member.build.completeDeps ++ member.build.completeBuildDeps ++ [ member.build ]
      ) (lib.attrValues crate2nix-builds.workspace.workspaceMembers);
    in
    attr: lib.unique (lib.concatMap (crate: crate.${attr} or [ ]) crates);

  # cargo compiles the whole workspace in one invocation here, so the per-crate
  # `env!()` variables all have to be in scope at once.
  crateEnvs = lib.foldl' (acc: envs: acc // envs) { } (
    lib.attrValues crate2nix-builds.crateEnvs
  );

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
    name = "flox-dev-crate2nix";

    inputsFrom = [ flox-nix-plugins ];

    buildInputs = crateInputs "buildInputs";

    nativeBuildInputs = crateInputs "nativeBuildInputs";

    packages = ciPackages ++ lib.optionals (!ci) devPackages;

    shellHook = pre-commit-check.shellHook + ''
      # Find the project root.
      REPO_ROOT="$( git rev-parse --show-toplevel; )";

      mkdir -p "$REPO_ROOT/build";
      rm -f "$REPO_ROOT/build/.env"; # clear the .env file
      rm -f "$REPO_ROOT/build/.PATH"; # clear the .PATH file
      cp -f ${envWrapper} "$REPO_ROOT/build/wrapper";


      # Define a environment variable and add it to the .env file.
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
      define_dev_env_var FLOX_BIN "''${REPO_ROOT}/target/debug/flox";
      define_dev_env_var FLOX_ACTIVATIONS_BIN "''${REPO_ROOT}/target/debug/flox-activations";

      # make built binaries
      define_dev_env_var BUILDENV_BIN "''${REPO_ROOT}/build/flox-buildenv/bin/buildenv";
      define_dev_env_var NIX_PLUGINS "''${REPO_ROOT}/build/nix-plugins/lib/nix-plugins";

      # static nix files
      define_dev_env_var FLOX_MK_CONTAINER_NIX "''${REPO_ROOT}/mkContainer/mkContainer.nix";

      # Nix built subsystems
      define_dev_env_var FLOX_INTERPRETER "''${REPO_ROOT}/build/flox-interpreter";
      define_dev_env_var FLOX_INTERPRETER_WRAPPER "''${REPO_ROOT}/build/flox-interpreter-build_executable_wrapper";
      define_dev_env_var FLOX_BUILDENV "''${REPO_ROOT}/build/flox-buildenv";
      define_dev_env_var FLOX_BUILDENV_NIX "''${FLOX_BUILDENV}/lib/buildenv.nix";
      define_dev_env_var FLOX_PACKAGE_BUILDER "''${REPO_ROOT}/build/flox-package-builder";
      define_dev_env_var FLOX_BUILD_MK "''${FLOX_PACKAGE_BUILDER}/libexec/flox-build.mk";
      define_dev_env_var FLOX_EXPRESSION_BUILD_NIX  "''${FLOX_PACKAGE_BUILDER}/libexec/nef/default.nix"
      define_dev_env_var FLOX_MANPAGES "''${REPO_ROOT}/build/flox-manpages";

      # test data
      define_dev_env_var INPUT_DATA "''${REPO_ROOT}/test_data/input_data";
      define_dev_env_var UNIT_TEST_GENERATED "''${REPO_ROOT}/test_data/unit_test_generated";
      define_dev_env_var GENERATED_DATA "''${REPO_ROOT}/test_data/generated";
      define_dev_env_var MANUALLY_GENERATED "''${REPO_ROOT}/test_data/manually_generated";
      define_dev_env_var FLOX_TEST_FIXED_STORE_PATH "${fixedTestStorePath}";

      # Add all internal rust crates to the PATH.
      # That's `flox` itself as well as the `flox-activations` subsystem.
      export PATH="''${REPO_ROOT}/target/debug":$PATH;
      echo -n "''${REPO_ROOT}/target/debug:" >> "$REPO_ROOT/build/.PATH";

      # Add the flox-manpages to the manpath
      export MANPATH="''${FLOX_MANPAGES}/share/man:$MANPATH"

      # configure the nix-plugin meson build
      meson setup --reconfigure --wipe \
      --prefix "''${REPO_ROOT}/build/nix-plugins" \
      "''${REPO_ROOT}/nix-plugins" "''${REPO_ROOT}/nix-plugins/builddir";

      echo;
      echo "run 'just build' to build flox and all its subsystems";
    '';
  }
  // crateEnvs
  // {
    RUST_SRC_PATH = "${rust-toolchain.rust-src}/lib/rustlib/src/rust/library";
    RUSTFMT = "${rustfmt}/bin/rustfmt";
  }
)
