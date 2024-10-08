{
  darwin,
  hostPlatform,
  inputs,
  lib,
  openssl,
  pkg-config,
  pkgsFor,
  rust-toolchain,
  flox-src,
}:
let
  # crane (<https://crane.dev/>) library for building rust packages
  craneLib = (inputs.crane.mkLib pkgsFor).overrideToolchain rust-toolchain.toolchain;

  # incremental build of third party crates
  cargoDepsArtifacts = craneLib.buildDepsOnly {
    pname = "flox-external-crates";

    # We don't want to version the 3rd party crates,
    #.to avoid cache invalidation.
    # In particular, we don't want to tie the version of the 3rd party crates
    # to the version of the flox-cli, as that means that we would have to
    # rebuild the 3rd party crates every time we update the flox-cli,
    # even though the dependencies of the flox-cli haven't changed.
    version = "unversioned";

    src = craneLib.cleanCargoSource (craneLib.path flox-src);

    # runtime dependencies of the dependent crates
    buildInputs =
      [
        # reqwest -> hyper -> openssl-sys
        openssl.dev
      ]
      ++ lib.optional hostPlatform.isDarwin [
        darwin.libiconv
        darwin.apple_sdk.frameworks.SystemConfiguration
      ];

    nativeBuildInputs = [ pkg-config ];
  };
in
cargoDepsArtifacts
