{
  inputs,
  gnused,
  lib,
  pkgsFor,
  rust-toolchain,
  rustfmt ? rust-toolchain.rustfmt,
  flox-src,
  rust-external-deps,
}:
let
  # crane (<https://crane.dev/>) library for building rust packages
  craneLib = (inputs.crane.mkLib pkgsFor).overrideToolchain rust-toolchain.toolchain;

in
craneLib.buildPackage ({
  pname = "flox-activations";
  version = lib.fileContents ./../../VERSION;
  src = flox-src;

  # Note about incremental compilation:
  #
  # Unlike the `flox` and `flox-watchdog` packages,
  # we cannot reuse the `flox-*-deps` packages for incremental compilation
  # because this crate is built with the "small" profile,
  # which among othet things applies different compiler optimizations.
  #
  # Crane will still cache the dependencies of this package,
  # through its own automation, but will experience cache misses
  # if we add shared (internal) packages.
  cargoExtraArgs = "--locked -p flox-activations";

  CARGO_LOG = "cargo::core::compiler::fingerprint=info";
  CARGO_PROFILE = "small";

  # runtime dependencies
  buildInputs = rust-external-deps.buildInputs ++ [ ];

  # build dependencies
  nativeBuildInputs = rust-external-deps.nativeBuildInputs;

  propagatedBuildInputs = rust-external-deps.propagatedBuildInputs ++ [ ];

  # https://github.com/ipetkov/crane/issues/385
  # doNotLinkInheritedArtifacts = true;

  # Tests are disabled inside of the build because the sandbox prevents
  # internet access and there are tests that require internet access to
  # resolve flake references among other things.
  doCheck = false;

  # bundle manpages and completion scripts
  #
  # sed: Removes rust-toolchain from binary. Likely due to toolchain overriding.
  #   unclear about the root cause, so this is a hotfix.
  postInstall = ''
    rm -f $out/bin/crane-*
    for target in "$(basename ${rust-toolchain.rust.outPath} | cut -f1 -d- )" ; do
      sed -i -e "s|$target|eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee|g" $out/bin/flox-activations
    done
  '';

  passthru = {
    ciPackages = [ ];

    devPackages = [ rustfmt ];

    devEnvs = {
      RUST_SRC_PATH = "${rust-toolchain.rust-src}/lib/rustlib/src/rust/library";
      RUSTFMT = "${rustfmt}/bin/rustfmt";
    };

    devShellHook = ''
      #  # Find the project root and add the `bin' directory to `PATH'.
      if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
        PATH="$( git rev-parse --show-toplevel; )/cli/target/debug":$PATH;
        REPO_ROOT="$( git rev-parse --show-toplevel; )";
      fi

    '';
  };
})
