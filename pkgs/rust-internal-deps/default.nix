{
  flox-pkgdb,
  gitMinimal,
  inputs,
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

    PROCESS_COMPOSE_BIN = "${process-compose}/bin/process-compose";

    GLOBAL_MANIFEST_TEMPLATE = builtins.path {
      path = ../../assets/global_manifest_template.toml;
    };

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
        cargoExtraArgs = "--locked -p flox -p klaus";

        # Compile the **non-dummy** lib crates only
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
