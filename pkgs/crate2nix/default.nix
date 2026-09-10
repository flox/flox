# Builds the workspace binaries from the generated `Cargo.nix` instead of
# crane. Kept next to the crane pipeline so the two can be compared before
# anything switches over; see CLI-164.
#
# crane builds the whole workspace in one derivation, so the compile-time
# `env!()` variables can be set once for that derivation. buildRustCrate
# compiles every crate separately, so each variable has to be attached to the
# crate that reads it. The `crateEnvs` inventory below is that mapping, and it
# doubles as a description of what a nixpkgs `buildRustPackage` build of flox
# would need.
{
  bash,
  cacert,
  coreutils,
  darwin,
  defaultCrateOverrides,
  flox-activations,
  flox-buildenv,
  flox-interpreter,
  flox-mk-container ? ../../mkContainer,
  flox-nix-plugins,
  flox-package-builder,
  gitMinimal,
  glibcLocalesUtf8,
  gnumake,
  inputs,
  krb5,
  lib,
  nix,
  nixpkgsInputLockedURL,
  pkg-config,
  pkgsFor,
  process-compose,
  rust-toolchain,
  rustPlatform,
  stdenv,
}:
let
  FLOX_VERSION = lib.fileContents ../../VERSION;

  auth0BaseUrl = "https://auth.flox.dev";

  # `env!()` variables grouped by the crate whose source reads them.
  # A missing variable fails the build of that crate with a compile error.
  crateEnvs = {
    flox-core = {
      NIXPKGS_CACERT_BUNDLE_CRT = cacert.outPath + "/etc/ssl/certs/ca-bundle.crt";
      inherit FLOX_VERSION;
    }
    // lib.optionalAttrs stdenv.hostPlatform.isDarwin {
      PATH_LOCALE = "${darwin.locale}/share/locale";
    }
    // lib.optionalAttrs stdenv.hostPlatform.isLinux {
      LOCALE_ARCHIVE = "${glibcLocalesUtf8}/lib/locale/locale-archive";
    };

    # The subsystems built outside cargo are nullable, as in
    # `pkgs/rust-internal-deps`: the development overlay passes `null` so that
    # entering the dev shell does not build them, and the shell exports the
    # mutable `build/` paths instead.
    flox-rust-sdk = {
      COMMON_NIXPKGS_URL = nixpkgsInputLockedURL inputs.nixpkgs;
      GIT_PKG = gitMinimal;
      GNUMAKE_BIN = "${gnumake}/bin/make";
      NIX_BIN = "${nix}/bin/nix";
      NIX_TARGET_SYSTEM = stdenv.targetPlatform.system;
      NIX_VERSION = nix.version;
      PROCESS_COMPOSE_BIN = "${process-compose}/bin/process-compose";
      SLEEP_BIN = "${coreutils}/bin/sleep";
      TESTING_BASE_CATALOG_URL = "https://github.com/flox/nixpkgs?rev=${inputs.nixpkgs.rev}";
    }
    // lib.optionalAttrs (flox-buildenv != null) {
      FLOX_BUILDENV_NIX = "${flox-buildenv}/lib/buildenv.nix";
    }
    // lib.optionalAttrs (flox-package-builder != null) {
      FLOX_BUILD_MK = "${flox-package-builder}/libexec/flox-build.mk";
      FLOX_EXPRESSION_BUILD_NIX = "${flox-package-builder}/libexec/nef/default.nix";
    }
    // lib.optionalAttrs (flox-nix-plugins != null) {
      NIX_PLUGINS = "${flox-nix-plugins}/lib/nix-plugins";
    }
    // lib.optionalAttrs (flox-mk-container != null) {
      FLOX_MK_CONTAINER_NIX = "${flox-mk-container}/mkContainer.nix";
    }
    // lib.optionalAttrs (flox-interpreter != null) {
      FLOX_INTERPRETER = flox-interpreter;
    };

    flox-manifest = {
      NIX_TARGET_SYSTEM = stdenv.targetPlatform.system;
    };

    flox = {
      INTERACTIVE_BASH_BIN = "${bash}/bin/bash";
      METRICS_EVENTS_URL = "https://z7qixlmjr3.execute-api.eu-north-1." + "amazonaws.com/prod/capture";
      METRICS_EVENTS_URL_V2 = "https://api.flox.dev/events";
      METRICS_EVENTS_API_KEY = "5pAQnBqz5Q7dpqVD9BEXQ4Kdc3D2fGTd3ZgP0XXK";
      METRICS_EVENTS_API_KEY_V2 = "pdCUpFGHGL5ytsYSdPJWP6MyUMnmUwN47mgsTIuX";
      NIX_TARGET_SYSTEM = stdenv.targetPlatform.system;
      OAUTH_CLIENT_ID = "fGrotHBfQr9X1PHGbFoifEWaDPyWZDmc";
      OAUTH_BASE_URL = auth0BaseUrl;
      OAUTH_TOKEN_URL = "${auth0BaseUrl}/oauth/token";
      OAUTH_DEVICE_AUTH_URL = "${auth0BaseUrl}/oauth/device/code";
    }
    // lib.optionalAttrs (flox-activations != null) {
      FLOX_ACTIVATIONS_BIN = "${flox-activations}/bin/flox-activations";
    };

    flox-activations = {
      COREUTILS = "${coreutils}";
      X_BASH_BIN = "${bash}/bin/bash";
    };

    nef-lock-catalog = {
      NIX_BIN = "${nix}/bin/nix";
    };
  };

  crateOverrides =
    defaultCrateOverrides
    // lib.mapAttrs (
      _: envs: _attrs:
      envs
    ) crateEnvs
    // {
      # libgssapi-sys generates its bindings with bindgen and finds the
      # Kerberos headers through pkg-config.
      libgssapi-sys = attrs: {
        nativeBuildInputs = (attrs.nativeBuildInputs or [ ]) ++ [
          pkg-config
          rustPlatform.bindgenHook
        ];
        buildInputs = (attrs.buildInputs or [ ]) ++ [ krb5.dev ];
      };
    };

  buildRustCrateForPkgs =
    pkgs: crateArgs:
    let
      buildRustCrate = pkgs.buildRustCrate.override {
        rustc = rust-toolchain.toolchain;
        cargo = rust-toolchain.toolchain;
        defaultCrateOverrides = crateOverrides;
      };
    in
    buildRustCrate (
      crateArgs
      // lib.optionalAttrs (stdenv.hostPlatform.system == "x86_64-linux") {
        extraRustcOpts = (crateArgs.extraRustcOpts or [ ]) ++ [ "-Clink-self-contained=-linker" ];
      }
    );

  workspace = import ../../Cargo.nix {
    pkgs = pkgsFor;
    inherit lib stdenv buildRustCrateForPkgs;
  };

  flox = workspace.workspaceMembers."flox".build;
  activations = workspace.workspaceMembers."flox-activations".build;
  nef-lock-catalog = workspace.workspaceMembers."nef-lock-catalog".build;
in
{
  # binary builds
  inherit flox nef-lock-catalog;
  flox-activations = activations;

  # The raw cargo workspace e.g. to build individual crates
  # and gather transitive native buildInputs
  inherit workspace;

  # The per-crate inventory of build time environments
  inherit crateEnvs;
}
