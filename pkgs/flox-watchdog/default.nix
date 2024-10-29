{
  inputs,
  gnused,
  pkgsFor,
  rust-toolchain,
  rustfmt ? rust-toolchain.rustfmt,
  rust-internal-deps,
  flox-src,
}:
let
  # crane (<https://crane.dev/>) library for building rust packages
  craneLib = (inputs.crane.mkLib pkgsFor).overrideToolchain rust-toolchain.toolchain;

  # build time environment variables
  envs = { } // rust-internal-deps.passthru.envs;
in
craneLib.buildPackage (
  {
    pname = "flox-watchdog";
    version = envs.FLOX_VERSION;
    src = flox-src;

    # Set up incremental compilation
    #
    # Cargo artifacts are built for the union of features used transitively
    # by `flox` and `flox-watchdog`.
    # Compiling either separately would result in a different set of features
    # and thus cache misses.
    cargoArtifacts = rust-internal-deps;
    cargoExtraArgs = "--locked -p flox-watchdog -p flox";
    postPatch = ''
      rm -rf ./flox/*
      cp -rf --no-preserve=mode ${craneLib.mkDummySrc { src = flox-src; }}/flox/* ./flox
    '';

    CARGO_LOG = "cargo::core::compiler::fingerprint=info";

    # runtime dependencies
    buildInputs = rust-internal-deps.buildInputs ++ [ ];

    # build dependencies
    nativeBuildInputs = rust-internal-deps.nativeBuildInputs ++ [ gnused ];

    propagatedBuildInputs = rust-internal-deps.propagatedBuildInputs ++ [ ];

    # https://github.com/ipetkov/crane/issues/385
    # doNotLinkInheritedArtifacts = true;

    # Tests are disabled inside of the build because the sandbox prevents
    # internet access and there are tests that require internet access to
    # resolve flake references among other things.
    doCheck = false;

    # bundle manpages and completion scripts
    #
    # mv: Moves to libexec to prevent it leaking onto PATH.
    #
    # sed: Removes rust-toolchain from binary. Likely due to toolchain overriding.
    #   unclear about the root cause, so this is a hotfix.
    postInstall = ''
      mv $out/bin $out/libexec
      rm -f $out/libexec/crane-*
      for target in "$(basename ${rust-toolchain.rust.outPath} | cut -f1 -d- )" ; do
        sed -i -e "s|$target|eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee|g" $out/libexec/flox-watchdog
      done
    '';

    passthru = {
      inherit envs;

      ciPackages = [ ];

      devPackages = [ rustfmt ];

      devEnvs = envs // {
        RUST_SRC_PATH = "${rust-toolchain.rust-src}/lib/rustlib/src/rust/library";
        RUSTFMT = "${rustfmt}/bin/rustfmt";
      };

      devShellHook = ''
        #  # Find the project root and add the `bin' directory to `PATH'.
        if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
          PATH="$( git rev-parse --show-toplevel; )/cli/target/debug":$PATH;
          REPO_ROOT="$( git rev-parse --show-toplevel; )";
          WATCHDOG_BIN="$REPO_ROOT/cli/target/debug/flox-watchdog";
        fi

      '';
    };
  }
  // envs
)
