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
  openssl,
  pkg-config,
  pkgsFor,
  process-compose,
  rust-toolchain,
  rustfmt ? rust-toolchain.rustfmt,
  targetPlatform,
}: let
  FLOX_VERSION = lib.fileContents ./../../VERSION;

  flox-src = builtins.path {
    name = "flox-src";
    path = "${./../../cli}";
    filter = path: type:
      ! builtins.elem path (map (
          f: "${./../../cli}/${f}"
        ) [
          "flake.nix"
          "flake.lock"
          "pkgs"
          "checks"
          "tests"
          "shells"
          "target"
        ]);
  };

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
      GIT_PKG = gitMinimal;
      NIX_PKG = nix;
      NIX_BIN = "${nix}/bin/nix"; # only used for nix invocations in tests
      PKGDB_BIN =
        if flox-pkgdb == null
        then "pkgdb"
        else "${flox-pkgdb}/bin/pkgdb";
      FLOX_ZDOTDIR = flox-activation-scripts + activate.d/zdotdir;
      PROCESS_COMPOSE_BIN = "${process-compose}/bin/process-compose";
      # [sic] nix handles `BASH_` variables specially,
      # so we need to use a different name.
      INTERACTIVE_BASH_BIN = "${bashInteractive}/bin/bash";

      # bundling of internally used nix scripts
      FLOX_RESOLVER_SRC = builtins.path {path = ../../resolver;};

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

      # the libssh crate wants to use its own libssh prebuilts
      # or build libssh from source.
      # This env variable will entcourage it to link to the nix provided version
      LIBSSH2_SYS_USE_PKG_CONFIG = "1";

      # used internally to ensure CA certificates are available
      NIXPKGS_CACERT_BUNDLE_CRT =
        cacert.outPath + "/etc/ssl/certs/ca-bundle.crt";

      # The current version of flox being built
      inherit FLOX_VERSION;

      # Reexport of the platform flox is being built for
      NIX_TARGET_SYSTEM = targetPlatform.system;

      # The global manifest we generate if one does not exist
      GLOBAL_MANIFEST_TEMPLATE = builtins.path {
        path = ../../assets/global_manifest_template.toml;
      };
    }
    // lib.optionalAttrs hostPlatform.isDarwin {
      NIX_COREFOUNDATION_RPATH = "${darwin.CF}/Library/Frameworks";
      PATH_LOCALE = "${darwin.locale}/share/locale";
    }
    // lib.optionalAttrs hostPlatform.isLinux {
      LOCALE_ARCHIVE = "${glibcLocalesUtf8}/lib/locale/locale-archive";
    };

  # incremental build of thrid party crates
  cargoDepsArtifacts = craneLib.buildDepsOnly {
    pname = "flox-cli";
    version = FLOX_VERSION;

    src = craneLib.cleanCargoSource (craneLib.path flox-src);

    # runtime dependencies of the dependent crates
    buildInputs =
      [
        # reqwest -> hyper -> openssl-sys
        openssl.dev
      ]
      ++ lib.optional hostPlatform.isDarwin [
        darwin.libiconv
        darwin.apple_sdk.frameworks.SystemConfiguration
      ];

    nativeBuildInputs = [
      pkg-config
    ];

    inherit (envs) LIBSSH2_SYS_USE_PKG_CONFIG;
  };
in
  craneLib.buildPackage ({
      pname = "flox-cli";
      version = envs.FLOX_VERSION;
      src = flox-src;

      cargoArtifacts = cargoDepsArtifacts;

      # runtime dependencies
      buildInputs = cargoDepsArtifacts.buildInputs ++ [];

      # build dependencies
      nativeBuildInputs =
        cargoDepsArtifacts.nativeBuildInputs
        ++ [
          installShellFiles
          gnused
        ];

      # https://github.com/ipetkov/crane/issues/385
      doNotLinkInheritedArtifacts = true;

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

        for target in "$(basename ${rust-toolchain.rust.outPath} | cut -f1 -d- )" ; do
          sed -i -e "s|$target|eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee|g" $out/bin/flox
          sed -i -e "s|$target|eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee|g" $out/bin/klaus
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
          rust-toolchain
          cargoDepsArtifacts
          pkgsFor
          nix
          flox-pkgdb
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
          fi

        '';
      };
    }
    // envs)
