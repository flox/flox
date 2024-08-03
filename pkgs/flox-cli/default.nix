{
  bashInteractive,
  cacert,
  darwin,
  flox-activation-scripts,
  flox-pkgdb,
  gitMinimal,
  glibcLocalesUtf8,
  gnused,
  hostPlatform,
  inputs,
  installShellFiles,
  lib,
  nix,
  pkgsFor,
  process-compose,
  rust-toolchain,
  rustfmt ? rust-toolchain.rustfmt,
  targetPlatform,
  rust-internal-deps,
  flox-klaus,
  flox-src,
}: let
  FLOX_VERSION = lib.fileContents ./../../VERSION;

  # crane (<https://crane.dev/>) library for building rust packages
  craneLib = (inputs.crane.mkLib pkgsFor).overrideToolchain rust-toolchain.toolchain;

  # build time environment variables
  envs = let
    auth0BaseUrl = "https://auth.flox.dev";
  in
    {
      # 3rd party CLIs
      # we want to use our own binaries by absolute path
      # rather than relying on or modifying the user's `PATH` variable
      NIX_BIN = "${nix}/bin/nix"; # only used for nix invocations in tests
      GIT_PKG = gitMinimal;

      KLAUS_BIN =
        if flox-klaus == null
        then "klaus"
        else "${flox-klaus}/bin/klaus";

      FLOX_ZDOTDIR = flox-activation-scripts + activate.d/zdotdir;

      # [sic] nix handles `BASH_` variables specially,
      # so we need to use a different name.
      INTERACTIVE_BASH_BIN = "${bashInteractive}/bin/bash";

      # Metrics subsystem configuration
      METRICS_EVENTS_URL =
        "https://z7qixlmjr3.execute-api.eu-north-1."
        + "amazonaws.com/prod/capture";
      METRICS_EVENTS_API_KEY = "5pAQnBqz5Q7dpqVD9BEXQ4Kdc3D2fGTd3ZgP0XXK";

      # oauth client id
      OAUTH_CLIENT_ID = "fGrotHBfQr9X1PHGbFoifEWaDPyWZDmc";
      OAUTH_BASE_URL = "${auth0BaseUrl}";
      OAUTH_AUTH_URL = "${auth0BaseUrl}/authorize";
      OAUTH_TOKEN_URL = "${auth0BaseUrl}/oauth/token";
      OAUTH_DEVICE_AUTH_URL = "${auth0BaseUrl}/oauth/device/code";

      # used internally to ensure CA certificates are available
      NIXPKGS_CACERT_BUNDLE_CRT =
        cacert.outPath + "/etc/ssl/certs/ca-bundle.crt";

      # The current version of flox being built
      inherit FLOX_VERSION;

      # Reexport of the platform flox is being built for
      NIX_TARGET_SYSTEM = targetPlatform.system;
    }
    // rust-internal-deps.passthru.envs
    // lib.optionalAttrs hostPlatform.isDarwin {
      NIX_COREFOUNDATION_RPATH = "${darwin.CF}/Library/Frameworks";
      PATH_LOCALE = "${darwin.locale}/share/locale";
    }
    // lib.optionalAttrs hostPlatform.isLinux {
      LOCALE_ARCHIVE = "${glibcLocalesUtf8}/lib/locale/locale-archive";
    };
in
  craneLib.buildPackage ({
      pname = "flox";
      version = envs.FLOX_VERSION;
      src = flox-src;

      # Set up incremental compilation
      #
      # Cargo artifacts are built for the union of features used transitively
      # by `flox` and `klaus`.
      # Compiling either separately would result in a different set of features
      # and thus cache misses.
      cargoArtifacts = rust-internal-deps;
      cargoExtraArgs = "--locked -p flox -p klaus";
      postPatch = ''
        rm -rf ./klaus/*
        cp -rf --no-preserve=mode ${craneLib.mkDummySrc {src = flox-src;}}/klaus/* ./klaus
      '';

      CARGO_LOG = "cargo::core::compiler::fingerprint=info";

      # runtime dependencies
      buildInputs = rust-internal-deps.buildInputs ++ [];

      # build dependencies
      nativeBuildInputs =
        rust-internal-deps.nativeBuildInputs
        ++ [
          installShellFiles
          gnused
        ];

      # https://github.com/ipetkov/crane/issues/385
      # doNotLinkInheritedArtifacts = true;

      # Tests are disabled inside of the build because the sandbox prevents
      # internet access and there are tests that require internet access to
      # resolve flake references among other things.
      doCheck = false;

      # bundle manpages and completion scripts
      #
      # sed: Removes rust-toolchain from binary. Likely due to toolchain overriding.
      #   unclear about the root cause, so this is a hotfix.
      postInstall = ''
        installShellCompletion --cmd flox                         \
          --bash <( "$out/bin/flox" --bpaf-complete-style-bash; ) \
          --fish <( "$out/bin/flox" --bpaf-complete-style-fish; ) \
          --zsh <( "$out/bin/flox" --bpaf-complete-style-zsh; );

        rm -f $out/bin/crane-*
        for target in "$(basename ${rust-toolchain.rust.outPath} | cut -f1 -d- )" ; do
          sed -i -e "s|$target|eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee|g" $out/bin/flox
        done
      '';

      doInstallCheck = false;
      postInstallCheck = ''
        # Quick unit test to ensure that we are not using any "naked"
        # commands within our scripts. Doesn't hit all codepaths but
        # catches most of them.
        : "''${USER:=$( id -un; )}";
        env -i USER="$USER" HOME="$PWD" "$out/bin/flox" --help > /dev/null;
        env -i USER="$USER" HOME="$PWD" "$out/bin/flox" nix help > /dev/null;
      '';

      passthru = {
        inherit
          envs
          flox-pkgdb
          flox-klaus
          ;

        ciPackages = [
          process-compose
        ];

        devPackages = [
          rustfmt
        ];

        devEnvs =
          envs
          // {
            RUST_SRC_PATH = "${rust-toolchain.rust-src}/lib/rustlib/src/rust/library";
            RUSTFMT = "${rustfmt}/bin/rustfmt";
          };

        devShellHook = ''
          #  # Find the project root and add the `bin' directory to `PATH'.
          if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
            PATH="$( git rev-parse --show-toplevel; )/cli/target/debug":$PATH;
            REPO_ROOT="$( git rev-parse --show-toplevel; )";
          fi

        '';
      };
    }
    // envs)
