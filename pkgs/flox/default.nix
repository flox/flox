{
  system,
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
  gnutar,
  zstd,
}: let
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

  envs =
    {
      NIX_BIN = "${flox-bash}/libexec/flox/nix";
      GIT_BIN = "${gitMinimal}/bin/git";
      FLOX_SH = "${flox-bash}/libexec/flox/flox";
      FLOX_SH_PATH = "${flox-bash}";
      FLOX_VERSION = "${cargoToml.package.version}-${inputs.flox-floxpkgs.lib.getRev self}";
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

  pname = cargoToml.package.name;
  version = envs.FLOX_VERSION;
  cargoLock = {
    lockFile = builtins.path {
      name = "Cargo.lock";
      path = flox-src + "/Cargo.lock";
    };
    allowBuiltinFetchGit = true;
  };
  buildType = "release";

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
  ];

  # Build dummy programs in order to populate a "./target" cache
  pre-build = rustPlatform.buildRustPackage {
    # Save cache directory
    postBuild = ''
      export SOURCE_DATE_EPOCH=1
      ${gnutar}/bin/tar --sort=name \
        --mtime="@''${SOURCE_DATE_EPOCH}" \
        --owner=0 \
        --group=0 \
        --numeric-owner \
        --pax-option=exthdr.name=%d/PaxHeaders/%f,delete=atime,delete=ctime \
        -c ./target | ${zstd}/bin/zstd "-T''${NIX_BUILD_CORES:-0}" -o $target
    '';
    outputs = ["out" "target"];
    inherit buildInputs nativeBuildInputs cargoLock buildType;
    name = pname + "-deps";
    cargoBuildFlags = ["--workspace"];
    preBuild = ''
      for i in crates/*/src; do
      cat <<'EOF' > "$i"/main.rs
      pub fn main() {}
      EOF
      done
    '';
    src = builtins.path {
      name = pname + "-src";
      path = flox-src;
      filter = path: type:
        (type == "directory")
        || (builtins.elem (builtins.baseNameOf path) ["Cargo.toml" "Cargo.lock"]);
    };
    doCheck = false;
    doInstallCheck = false;
  };

  drv = rustPlatform.buildRustPackage ({
      inherit buildInputs nativeBuildInputs cargoLock buildType;
      inherit pname version;
      src = flox-src;

      # Extract cache directory
      preBuild = ''
        mkdir ./target
        ${zstd}/bin/zstd -d "${pre-build.target}" --stdout | \
          ${gnutar}/bin/tar -x -C ./. --strip-components=1
        # TODO: use a better date?
        find "./target" -exec touch -cfhd "$(date --date=tomorrow)" -- {} +
      '';

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
        env -i USER=`id -un` HOME=$PWD $out/bin/flox nix help > /dev/null
      '';

      passthru.envs = envs;
      passthru.manpages = manpages;
      passthru.rustPlatform = rustPlatform;
      passthru.flox-bash = flox-bash;
      passthru.pre-build = pre-build;
    }
    // envs);
in
  drv
