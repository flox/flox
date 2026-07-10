{
  cacert,
  glibcLocalesUtf8,
  inputs,
  lib,
  nix,
  pkgsFor,
  rust-toolchain,
  rustfmt ? rust-toolchain.rustfmt,
  flox-src,
  rust-external-deps,
  stdenv,
  # Override catalog authentication strategy if needed
  # Options: "floxhub-authn-kerberos"
  overrideCatalogAuth ? null,
}:
let
  FLOX_VERSION = lib.fileContents ./../../VERSION;
  # crane (<https://crane.dev/>) library for building rust packages
  craneLib = (inputs.crane.mkLib pkgsFor).overrideToolchain rust-toolchain.toolchain;
  envs = {
    # used internally to ensure CA certificates are available
    NIXPKGS_CACERT_BUNDLE_CRT = cacert.outPath + "/etc/ssl/certs/ca-bundle.crt";
    # `flox-core` (a transitive dependency) reads this with `env!` at compile
    # time. This binary does not activate environments, but the variable must
    # be defined for the build to compile, so point it at this package's own
    # output as flox-activations does.
    FLOX_ACTIVATIONS_BIN = "${placeholder "out"}/libexec/flox-activations";
    # `nef-lock-catalog` reads this with `env!` at compile time to locate `nix`.
    NIX_BIN = "${nix}/bin/nix";
    FLOX_VERSION = FLOX_VERSION;
  };

in
craneLib.buildPackage (
  {
    pname = "nef-lock-catalog";
    version = lib.fileContents ./../../VERSION;
    src = flox-src;

    # Note about incremental compilation:
    #
    # Unlike the `flox` package,
    # we cannot reuse the `flox-*-deps` packages for incremental compilation
    # because this crate is built with the "small" profile,
    # which among other things applies different compiler optimizations.
    #
    # Crane will still cache the dependencies of this package,
    # through its own automation, but will experience cache misses
    # if we add shared (internal) packages.
    #
    # `nef-lock-catalog` is not a cargo default-member, so it must be built
    # explicitly with `-p`.
    cargoExtraArgs =
      "--locked -p nef-lock-catalog"
      + lib.optionalString (overrideCatalogAuth != null) " --features ${overrideCatalogAuth}";

    CARGO_PROFILE = "small";

    # runtime dependencies
    buildInputs = rust-external-deps.buildInputs;

    # build dependencies
    nativeBuildInputs = rust-external-deps.nativeBuildInputs;

    propagatedBuildInputs = rust-external-deps.propagatedBuildInputs ++ [ ];

    # Tests are disabled inside of the build because the sandbox prevents
    # internet access and there are tests that require internet access to
    # resolve flake references among other things.
    doCheck = false;

    # The binary is named `lock` (from `src/bin/lock.rs`); install it as a
    # libexec binary so the package builder can reference it by absolute path.
    #
    # sed: Removes rust-toolchain from binary. Likely due to toolchain
    #   overriding. Unclear about the root cause, so this is a hotfix.
    postInstall = ''
      rm -f $out/bin/crane-*
      for target in "$(basename ${rust-toolchain.rust.outPath} | cut -f1 -d- )" ; do
        sed -i -e "s|$target|eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee|g" $out/bin/lock
      done
      mv $out/bin $out/libexec
    '';

    passthru = {
      ciPackages = [ ];

      devPackages = [ rustfmt ];

      devEnvs = {
        RUST_SRC_PATH = "${rust-toolchain.rust-src}/lib/rustlib/src/rust/library";
        RUSTFMT = "${rustfmt}/bin/rustfmt";
      }
      // envs;
    };
  }
  // rust-external-deps.passthru.envs
  // envs
  // lib.optionalAttrs stdenv.hostPlatform.isLinux {
    LOCALE_ARCHIVE = "${glibcLocalesUtf8}/lib/locale/locale-archive";
  }
)
