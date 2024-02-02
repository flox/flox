{
  inputs,
  lib,
  clippy,
  rust-analyzer,
  rust,
  rustc,
  rustfmt,
  rustPlatform,
  hostPlatform,
  targetPlatform,
  openssl,
  libssh2,
  libgit2,
  zlib,
  pkg-config,
  darwin,
  parser-util,
  pandoc,
  cacert,
  glibcLocalesUtf8,
  installShellFiles,
  runCommand,
  fd,
  gnused,
  gitMinimal,
  nix,
  pkgsFor,
  flox-pkgdb,
}: let
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
  craneLib = inputs.crane.mkLib pkgsFor;

  # build time environment variables
  envs = let
    auth0BaseUrl = "https://flox-dev.us.auth0.com";
  in
    {
      # 3rd party CLIs
      # we want to use our own binaries by absolute path
      # rather than relying on or modifying the user's `PATH` variable
      GIT_BIN = "${gitMinimal}/bin/git";
      NIX_BIN = "${nix}/bin/nix";
      PKGDB_BIN =
        if flox-pkgdb == null
        then "pkgdb"
        else "${flox-pkgdb}/bin/pkgdb";
      LD_FLOXLIB =
        if flox-pkgdb == null
        then "ld-floxlib.so"
        else "${flox-pkgdb}/lib/ld-floxlib.so";
      PARSER_UTIL_BIN = "${parser-util}/bin/parser-util";
      FLOX_ETC_DIR = ../../assets/etc;
      FLOX_ZDOTDIR = ../../assets/flox.zdotdir;

      # bundling of internally used nix scripts
      FLOX_RESOLVER_SRC = builtins.path {path = ../../resolver;};

      # Metrics subsystem configuration
      METRICS_EVENTS_URL =
        "https://z7qixlmjr3.execute-api.eu-north-1."
        + "amazonaws.com/prod/capture";
      METRICS_EVENTS_API_KEY = "5pAQnBqz5Q7dpqVD9BEXQ4Kdc3D2fGTd3ZgP0XXK";

      # oauth client id
      OAUTH_CLIENT_ID = "e8BGlekBE8w88LyrqGifts588wHtw03Q";
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
      FLOX_VERSION = cargoToml.package.version;

      # Reexport of the platform flox is being built for
      NIX_TARGET_SYSTEM = targetPlatform.system;

      # flox env template used to create new environments
      FLOX_ENV_TEMPLATE = builtins.path {
        path = ../../assets/templateFloxEnv;
      };

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

  # compiled manpages
  manpages =
    runCommand "flox-manpages" {
      src = flox-src + "/flox/doc";
      buildInputs = [pandoc fd];
    } ''

      mkdir $out
      pushd $src

      fd "flox.*.md" ./ -x \
        pandoc -t man \
          -L ${./pandoc-filters/include-files.lua} \
          --standalone \
          -o "$out/{/.}.1" \
          {}
    '';

  cargoToml = lib.importTOML (flox-src + "/flox/Cargo.toml");

  # incremental build of thrid party crates
  cargoDepsArtifacts = craneLib.buildDepsOnly {
    pname = cargoToml.package.name;
    version = cargoToml.package.version;
    src = craneLib.cleanCargoSource (craneLib.path flox-src);

    # runtime dependencies of the dependent crates
    buildInputs =
      [
        openssl.dev # octokit -> hyper -> ssl
        zlib # git2
        libssh2 # git2
        libgit2 # git2
      ]
      ++ lib.optional hostPlatform.isDarwin [
        darwin.apple_sdk.frameworks.Security # git2 (and others)
        darwin.apple_sdk.frameworks.SystemConfiguration
      ];

    nativeBuildInputs = [
      pkg-config # for openssl
    ];

    inherit (envs) LIBSSH2_SYS_USE_PKG_CONFIG PARSER_UTIL_BIN;
  };
in
  craneLib.buildPackage ({
      pname = cargoToml.package.name;
      version = envs.FLOX_VERSION;
      src = flox-src;

      cargoArtifacts = cargoDepsArtifacts;

      outputs = ["out" "man"];
      outputsToInstall = ["out" "man"];

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
      postInstall = ''
        ln -s "${envs.FLOX_ETC_DIR}" "$out/etc"
        installManPage ${manpages}/*;
        installShellCompletion --cmd flox                         \
          --bash <( "$out/bin/flox" --bpaf-complete-style-bash; ) \
          --fish <( "$out/bin/flox" --bpaf-complete-style-fish; ) \
          --zsh <( "$out/bin/flox" --bpaf-complete-style-zsh; );
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
          manpages
          rustPlatform
          cargoDepsArtifacts
          pkgsFor
          nix
          flox-pkgdb
          ;

        ciPackages = [];

        devPackages = [
          rustfmt
          clippy
          rust-analyzer
          rust.packages.stable.rustPlatform.rustLibSrc
          rustc
        ];

        devEnvs =
          envs
          // {
            RUST_SRC_PATH = rustPlatform.rustLibSrc.outPath;
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
