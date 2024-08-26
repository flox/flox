{
  flox-pkgdb,
  gitMinimal,
  inputs,
  coreutils,
  lib,
  pkgsFor,
  process-compose,
  rust-toolchain,
  targetPlatform,
  rust-external-deps,
  flox-src,
}: let
  FLOX_VERSION = lib.fileContents ./../../VERSION;

  # crane (<https://crane.dev/>) library for building rust packages
  craneLib = (inputs.crane.mkLib pkgsFor).overrideToolchain rust-toolchain.toolchain;

  # build time environment variables
  envs = {
    # 3rd party CLIs
    # we want to use our own binaries by absolute path
    # rather than relying on or modifying the user's `PATH` variable
    GIT_PKG = gitMinimal;
    PKGDB_BIN =
      if flox-pkgdb == null
      then "pkgdb"
      else "${flox-pkgdb}/bin/pkgdb";

    SLEEP_BIN = "${coreutils}/bin/sleep";

    PROCESS_COMPOSE_BIN = "${process-compose}/bin/process-compose";

    # The current version of flox being built
    inherit FLOX_VERSION;

    # Reexport of the platform flox is being built for
    NIX_TARGET_SYSTEM = targetPlatform.system;
  };
in
  (craneLib.buildDepsOnly
    ({
        pname = "flox-internal-deps";
        version = envs.FLOX_VERSION;
        src = flox-src;

        # `buildDepsOnly` replaces the source of _all_ crates in the workspace
        # with "dummy" packages, essentially empty {lib,main}.rs files.
        # The effect is that cargo will build all required dependencies
        # but not the actual crates in the workspace -- hence "depsOnly".
        # In this case we do want to build some of the crates in the workspace,
        # i.e. flox-rust-sdk and catalog-api-v1 as dependencies of flox and flox-watchdog.
        # To achieve this, we copy the source of these crates back into the workspace.
        cargoExtraArgs = "--locked -p flox -p flox-watchdog";
        postPatch = ''
          cp -rf --no-preserve=mode ${flox-src}/flox-rust-sdk/* ./flox-rust-sdk
          cp -rf --no-preserve=mode ${flox-src}/catalog-api-v1/* ./catalog-api-v1
        '';

        # runtime dependencies
        buildInputs =
          rust-external-deps.buildInputs
          ++ [];

        # build dependencies
        nativeBuildInputs = rust-external-deps.nativeBuildInputs ++ [];

        propagatedBuildInputs =
          rust-external-deps.propagatedBuildInputs
          ++ [
            gitMinimal
            process-compose
            coreutils # for `sleep infinity`
          ]
          ++ lib.optional (flox-pkgdb != null) [
            flox-pkgdb
          ];

        # Tests are disabled inside of the build because the sandbox prevents
        # internet access and there are tests that require internet access to
        # resolve flake references among other things.
        doCheck = false;

        passthru = {
          inherit envs;
        };
      }
      // envs))
  .overrideAttrs (oldAttrs: {
    # avoid rebuilding 3rd party deps
    cargoArtifacts = rust-external-deps;
  })
