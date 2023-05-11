{
  system,
  # self is a flake if this package is built locally, but if it's called as a proto, it's just the
  # source
  self,
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
  bats,
  gitMinimal,
}: let
  manpages =
    runCommand "flox-manpages" {
      src = "${self}/crates/flox/doc";
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

  cargoToml = lib.importTOML (self + "/crates/flox/Cargo.toml");

  envs =
    {
      NIX_BIN = "${flox-bash.nixPatched}/bin/nix";
      GIT_BIN = "${gitMinimal}/bin/git";
      FLOX_SH = "${flox-bash}/libexec/flox/flox";
      FLOX_SH_PATH = "${flox-bash}";
      FLOX_SH_FLAKE = flox-bash.src; # For bats tests
      FLOX_VERSION = "${cargoToml.package.version}-r${self.lib.getRev self}";
      NIXPKGS_CACERT_BUNDLE_CRT = "${cacert}/etc/ssl/certs/ca-bundle.crt";
      NIX_TARGET_SYSTEM = targetPlatform.system;

      NIX_BASH_COMPLETION_SCRIPT = ../../crates/flox/src/static/nix_bash_completion.sh;
      NIX_ZSH_COMPLETION_SCRIPT = ../../crates/flox/src/static/nix_zsh_completion.sh;

      FLOX_RESOLVER_SRC = ../../resolver;

      METRICS_EVENTS_URL = "https://events.floxdev.com/capture";
      METRICS_EVENTS_API_KEY = "phc_z4dOADAPvpU9VNzCjDD3pIJuSuGTyagKdFWfjak838Y";

      LIBSSH2_SYS_USE_PKG_CONFIG = "1";
    }
    // lib.optionalAttrs hostPlatform.isDarwin {
      NIX_COREFOUNDATION_RPATH = "${darwin.CF}/Library/Frameworks";
      PATH_LOCALE = "${darwin.locale}/share/locale";
    }
    // lib.optionalAttrs hostPlatform.isLinux {
      LOCALE_ARCHIVE = "${glibcLocales}/lib/locale/locale-archive";
    };
in
  rustPlatform.buildRustPackage ({
      pname = cargoToml.package.name;
      version = envs.FLOX_VERSION;
      src = self;

      cargoLock = {
        lockFile = self + "/Cargo.lock";
        allowBuiltinFetchGit = true;
      };

      outputs = ["out" "man"];
      outputsToInstall = ["out" "man"];

      buildAndTestSubdir = "crates/flox";

      doCheck = true;
      cargoTestFlags = ["--workspace"];

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
      '';

      buildInputs =
        [
          openssl.dev
          zlib
          libssh2
          libgit2
        ]
        ++ lib.optional hostPlatform.isDarwin [
          darwin.apple_sdk.frameworks.Security
        ];

      nativeBuildInputs = [
        pkg-config # for openssl
        pandoc
        installShellFiles
        gnused
        (bats.withLibraries (p: [p.bats-support p.bats-assert]))
      ];

      passthru.envs = envs;
      passthru.manpages = manpages;
      passthru.rustPlatform = rustPlatform;
      passthru.flox-bash = flox-bash;
    }
    // envs)
