{
  nixpkgs,
  # self is a flake if this package is built locally, but if it's called as a proto, it's just the
  # source
  self,
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
  pandoc,
  cacert,
  glibcLocales,
  installShellFiles,
  runCommand,
  fd,
  gnused,
  gitMinimal,
}: let
  # crane (<https://crane.dev/>) library for building rust packages
  craneLib = inputs.crane.mkLib nixpkgs;

  # build time environment variables
  envs =
    {
      # 3rd party CLIs
      # we want to use our own binaries by absolute path
      # rather than relying on or modifying the user's `PATH` variable
      NIX_BIN = "${flox-bash}/libexec/flox/nix";
      GIT_BIN = "${gitMinimal}/bin/git";

      # path to bash impl of flox to dispatch unimplemented commands to
      FLOX_SH = "${flox-bash}/libexec/flox/flox";
      FLOX_SH_PATH = "${flox-bash}";

      # Modified nix completion scripts
      # used to pass through nix completion ability for `flox nix *`
      NIX_BASH_COMPLETION_SCRIPT = ../../crates/flox/src/static/nix_bash_completion.sh;
      NIX_ZSH_COMPLETION_SCRIPT = ../../crates/flox/src/static/nix_zsh_completion.sh;

      # bundling of an internally used nix scripts
      FLOX_RESOLVER_SRC = ../../resolver;
      FLOX_ANALYZER_SRC = ../../flox-bash/lib/catalog-ingest;

      # Metrics subsystem configuration
      METRICS_EVENTS_URL = "https://events.floxdev.com/capture";
      METRICS_EVENTS_API_KEY = "phc_z4dOADAPvpU9VNzCjDD3pIJuSuGTyagKdFWfjak838Y";

      # the libssh crate wants to use its own libssh prebuilts
      # or build libssh from source.
      # This env variable will entcourage it to link to the nix provided version
      LIBSSH2_SYS_USE_PKG_CONFIG = "1";

      # used internally to ensure CA certificates are available
      NIXPKGS_CACERT_BUNDLE_CRT = "${cacert}/etc/ssl/certs/ca-bundle.crt";

      # The current version of flox being built
      FLOX_VERSION = "${cargoToml.package.version}-${inputs.flox-floxpkgs.lib.getRev self}";
      # Reexport of the platform flox is being built for
      NIX_TARGET_SYSTEM = targetPlatform.system;
    }
    // lib.optionalAttrs hostPlatform.isDarwin {
      NIX_COREFOUNDATION_RPATH = "${darwin.CF}/Library/Frameworks";
      PATH_LOCALE = "${darwin.locale}/share/locale";
    }
    // lib.optionalAttrs hostPlatform.isLinux {
      LOCALE_ARCHIVE = "${glibcLocales}/lib/locale/locale-archive";
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
      ];

    nativeBuildInputs = [
      pkg-config # for openssl
    ];

    inherit (envs) LIBSSH2_SYS_USE_PKG_CONFIG;
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

      # test all our crates (include the libraries)
      cargoTestExtraArgs = "--workspace";

      # bundle manpages and completion scripts
      postInstall = ''
        installManPage ${manpages}/*
        installShellCompletion --cmd flox \
          --bash <($out/bin/flox --bpaf-complete-style-bash) \
          --fish <($out/bin/flox --bpaf-complete-style-fish) \
          --zsh <($out/bin/flox --bpaf-complete-style-zsh)
      '';

      doInstallCheck = true;
      postInstallCheck = ''
        # Quick unit test to ensure that we are not using any "naked"
        # commands within our scripts. Doesn't hit all codepaths but
        # catches most of them.
        env -i USER=`id -un` HOME=$PWD $out/bin/flox --debug envs > /dev/null
        env -i USER=`id -un` HOME=$PWD $out/bin/flox nix help > /dev/null
      '';

      passthru.envs = envs;
      passthru.manpages = manpages;
      passthru.rustPlatform = rustPlatform;
      passthru.flox-bash = flox-bash;
      passthru.cargoDeps = cargoDepsArtifacts;
    }
    // envs)
