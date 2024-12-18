{
  coreutils,
  flox-buildenv,
  flox-package-builder,
  flox-mk-container ? ../../mkContainer,
  flox-pkgdb,
  flox-src,
  gitMinimal,
  gnumake,
  inputs,
  lib,
  nix,
  pkgsFor,
  process-compose,
  rust-external-deps,
  rust-toolchain,
  targetPlatform,
}:
let
  FLOX_VERSION = lib.fileContents ./../../VERSION;

  # crane (<https://crane.dev/>) library for building rust packages
  craneLib = (inputs.crane.mkLib pkgsFor).overrideToolchain rust-toolchain.toolchain;

  # build time environment variables
  envs =
    {
      # 3rd party CLIs
      # we want to use our own binaries by absolute path
      # rather than relying on or modifying the user's `PATH` variable
      GIT_PKG = gitMinimal;
      NIX_BIN = "${nix}/bin/nix";
      GNUMAKE_BIN = "${gnumake}/bin/make";
      SLEEP_BIN = "${coreutils}/bin/sleep";
      PROCESS_COMPOSE_BIN = "${process-compose}/bin/process-compose";

      # Used by `flox build' to access `stdenv` at a known version
      # When utilities from nixpkgs are used by flox at runtime,
      # they should be
      # a) bundled at buildtime if possible (binaries/packages)
      # b) use this version of nixpkgs i.e. (nix library utils such as `lib` and `runCommand`)
      COMMON_NIXPKGS_URL = "path:${inputs.nixpkgs.outPath}";

      # The current version of flox being built
      inherit FLOX_VERSION;

      # Reexport of the platform flox is being built for
      NIX_TARGET_SYSTEM = targetPlatform.system;
    }
    # Our own tools
    # In the dev shell these will be set dynamically
    // lib.optionalAttrs (flox-buildenv != null) {
      FLOX_BUILDENV_NIX = "${flox-buildenv}/lib/buildenv.nix";
    }
    // lib.optionalAttrs (flox-package-builder != null) {
      FLOX_BUILD_MK = "${flox-package-builder}/libexec/flox-build.mk";
    }
    // lib.optionalAttrs (flox-pkgdb != null) {
      PKGDB_BIN = "${flox-pkgdb}/bin/flox-pkgdb";
    }
    // lib.optionalAttrs (flox-mk-container != null) {
      FLOX_MK_CONTAINER_NIX = "${flox-mk-container}/mkContainer.nix";
    };

in
(craneLib.buildDepsOnly (
  {
    pname = "flox-internal-deps";
    version = envs.FLOX_VERSION;
    src = flox-src;

    # `buildDepsOnly` replaces the source of _all_ crates in the workspace
    # with "dummy" packages, essentially empty {lib,main}.rs files.
    # The effect is that cargo will build all required dependencies
    # but not the actual crates in the workspace -- hence "depsOnly".
    # In this case we do want to build some of the crates in the workspace,
    # i.e. flox-rust-sdk, catalog-api-v1, and shared as dependencies of flox
    # and flox-watchdog.
    # To achieve this, we copy the source of these crates back into the workspace.
    cargoExtraArgs = "--locked -p flox -p flox-watchdog";
    postPatch = ''
      cp -rf --no-preserve=mode ${flox-src}/flox-rust-sdk/* ./flox-rust-sdk
      cp -rf --no-preserve=mode ${flox-src}/catalog-api-v1/* ./catalog-api-v1
      cp -rf --no-preserve=mode ${flox-src}/flox-core/* ./flox-core
      cp -rf --no-preserve=mode ${flox-src}/flox-test-utils/* ./flox-test-utils
    '';

    # runtime dependencies
    buildInputs = rust-external-deps.buildInputs ++ [ ];

    # build dependencies
    nativeBuildInputs = rust-external-deps.nativeBuildInputs ++ [ ];

    propagatedBuildInputs =
      rust-external-deps.propagatedBuildInputs
      ++ [
        gitMinimal
        process-compose
        coreutils # for `sleep infinity`
      ]
      ++ lib.optional (flox-pkgdb != null) [ flox-pkgdb ]
      ++ lib.optional (flox-mk-container != null) [ flox-mk-container ];

    # Tests are disabled inside of the build because the sandbox prevents
    # internet access and there are tests that require internet access to
    # resolve flake references among other things.
    doCheck = false;

    passthru = {
      inherit envs;
    };
  }
  // envs
)).overrideAttrs
  (oldAttrs: {
    # avoid rebuilding 3rd party deps
    cargoArtifacts = rust-external-deps;
  })
