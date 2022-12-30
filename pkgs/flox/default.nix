{
  system,
  # self is a flake if this package is built localy, but if it's called as a proto, it's just the
  # source
  self,
  lib,
  rustPlatform,
  hostPlatform,
  targetPlatform,
  openssl,
  pkg-config,
  darwin,
  flox-bash ?
    self.inputs.flox-floxpkgs-internal.packages.${system}.flox-bash-private
    or self.inputs.flox-floxpkgs-internal.packages.${system}.flox-internal,
  nixStable,
  pandoc,
  cacert,
  glibcLocales,
  installShellFiles,
  runCommand,
  fd,
  gnused,
  bats,
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
  nix = nixStable.overrideAttrs (oldAttrs: {
    patches =
      (oldAttrs.patches or [])
      ++ [
        ./nix-patches/CmdProfileBuild.patch
        ./nix-patches/CmdSearchAttributes.patch
        ./nix-patches/update-profile-list-warning.patch
        ./nix-patches/multiple-github-tokens.patch
        ./nix-patches/curl_flox_version.patch
      ];
  });

  envs =
    {
      NIX_BIN = "${nix}/bin/nix";
      FLOX_SH = "${flox-bash}/libexec/flox/flox";
      FLOX_SH_PATH = "${flox-bash}";
      FLOX_SH_FLAKE = flox-bash.src; # For bats tests
      FLOX_VERSION = "${envs.FLOX_RS_VERSION}-${envs.FLOX_SH_VERSION}";
      FLOX_SH_VERSION = flox-bash.version;
      FLOX_RS_VERSION = "${cargoToml.package.version}-r${toString self.revCount or "dirty"}";
      NIXPKGS_CACERT_BUNDLE_CRT = "${cacert}/etc/ssl/certs/ca-bundle.crt";
      NIX_TARGET_SYSTEM = targetPlatform.system;

      NIX_BASH_COMPLETION_SCRIPT = ../../crates/flox/src/static/nix_bash_completion.sh;
      NIX_ZSH_COMPLETION_SCRIPT = ../../crates/flox/src/static/nix_zsh_completion.sh;

      FLOX_RESOLVER_SRC = ../../resolver;

      # Static "floxbeta" token for closed beta.
      # XXX Remove after closed beta.
      BETA_ACCESS_TOKEN = "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";
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
      };

      outputs = ["out" "man"];
      outputsToInstall = ["out" "man"];

      buildAndTestSubdir = "crates/flox";

      doCheck = false;

      postInstall = ''
        installManPage ${manpages}/*
        installShellCompletion --cmd flox \
          --bash <($out/bin/flox --bpaf-complete-style-bash) \
          --fish <($out/bin/flox --bpaf-complete-style-fish) \
          --zsh <($out/bin/flox --bpaf-complete-style-zsh)
      '';

      buildInputs =
        [
          openssl.dev
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
    }
    // envs)
