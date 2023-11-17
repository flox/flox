{
  flox-src,
  inputs,
  lib,
  rustPlatform,
  hostPlatform,
  targetPlatform,
  openssl,
  libssh2,
  libgit2,
  zlib,
  pkg-config,
  darwin,
  flox-bash,
  parser-util,
  pandoc,
  cacert,
  glibcLocalesUtf8,
  installShellFiles,
  runCommand,
  fd,
  gnused,
  gitMinimal,
  flox-gh,
  gh,
  pkgsFor,
  floxVersion,
  flox-pkgdb,
}: let
  # crane (<https://crane.dev/>) library for building rust packages
  craneLib = inputs.crane.mkLib pkgsFor;

  # build time environment variables
  envs = let
    # we need to pull all of the scripts in the mkEnv directory into /nix/store
    mkEnv = ../../assets/mkEnv;
  in
    {
      # 3rd party CLIs
      # we want to use our own binaries by absolute path
      # rather than relying on or modifying the user's `PATH` variable
      NIX_BIN = "${flox-bash}/libexec/flox/nix";
      GIT_BIN = "${gitMinimal}/bin/git";
      PARSER_UTIL_BIN = "${parser-util}/bin/parser-util";
      PKGDB_BIN = "${flox-pkgdb}/bin/pkgdb";
      FLOX_GH_BIN = "${flox-gh}/bin/flox-gh";
      GH_BIN = "${gh}/bin/gh";
      FLOX_SH_PATH = flox-bash.outPath;
      ENV_FROM_LOCKFILE_PATH = "${mkEnv}/env-from-lockfile.nix";
      BUILD_ENV_BIN = ../../assets/build-env.sh;

      # Modified nix completion scripts
      # used to pass through nix completion ability for `flox nix *`
      NIX_BASH_COMPLETION_SCRIPT =
        ../../crates/flox/src/static/nix_bash_completion.sh;
      NIX_ZSH_COMPLETION_SCRIPT =
        ../../crates/flox/src/static/nix_zsh_completion.sh;

      # bundling of internally used nix scripts
      FLOX_RESOLVER_SRC = builtins.path {path = ../../resolver;};

      # Metrics subsystem configuration
      METRICS_EVENTS_URL = "https://events.flox.dev/capture";
      METRICS_EVENTS_API_KEY = "phc_z4dOADAPvpU9VNzCjDD3pIJuSuGTyagKdFWfjak838Y";

      # oauth client id
      OAUTH_CLIENT_ID = "Iv1.3b00a7bb5f910259";

      # the libssh crate wants to use its own libssh prebuilts
      # or build libssh from source.
      # This env variable will entcourage it to link to the nix provided version
      LIBSSH2_SYS_USE_PKG_CONFIG = "1";

      # used internally to ensure CA certificates are available
      NIXPKGS_CACERT_BUNDLE_CRT = cacert.outPath + "/etc/ssl/certs/ca-bundle.crt";

      # The current version of flox being built
      FLOX_VERSION = floxVersion;
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
      src = flox-src + "/crates/flox/doc";
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

  cargoToml = lib.importTOML (flox-src + "/crates/flox/Cargo.toml");

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

      passthru.envs = envs;
      passthru.manpages = manpages;
      passthru.rustPlatform = rustPlatform;
      passthru.flox-bash = flox-bash;
      passthru.cargoDeps = cargoDepsArtifacts;
    }
    // envs)
