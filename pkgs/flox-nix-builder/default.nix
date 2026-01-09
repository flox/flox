{
  inputs,
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
  pname = "flox-nix-builder";
  version = lib.fileContents ./../../VERSION;
  src = flox-src;

  # Build only the nix-builder package
  cargoExtraArgs = "--locked -p nix-builder";

  CARGO_LOG = "cargo::core::compiler::fingerprint=info";

  # runtime dependencies
  buildInputs = rust-external-deps.buildInputs ++ [ ];

  # build dependencies
  nativeBuildInputs = rust-external-deps.nativeBuildInputs;

  propagatedBuildInputs = rust-external-deps.propagatedBuildInputs ++ [ ];

  # Tests are disabled inside of the build because the sandbox prevents
  # internet access and there are tests that require internet access to
  # resolve flake references among other things.
  doCheck = false;

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
        PATH="$( git rev-parse --show-toplevel; )/cli/target/release":$PATH;
        REPO_ROOT="$( git rev-parse --show-toplevel; )";
      fi
    '';
  };
})
