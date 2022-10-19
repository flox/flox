{
  self,
  lib,
  rustPlatform,
  hostPlatform,
  openssl,
  pkg-config,
  darwin,
}:
rustPlatform.buildRustPackage rec {
  pname = "flox-cli";
  version = "0.0.0";
  src = self;

  cargoLock = {
    lockFile = self + "/Cargo.lock";
  };

  buildAndTestSubdir = "crates/flox-cli";

  doCheck = false;

  buildInputs =
    [
      openssl.dev
    ]
    ++ lib.optional hostPlatform.isDarwin [
      darwin.apple_sdk.frameworks.Security
    ];

  nativeBuildInputs = [
    pkg-config # for openssl
  ];
}
