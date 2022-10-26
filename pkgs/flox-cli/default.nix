{
  self,
  lib,
  rustPlatform,
  hostPlatform,
  openssl,
  pkg-config,
  darwin,
}: let
  cargoToml = lib.importTOML (self + "/crates/flox-cli/Cargo.toml");
in
  rustPlatform.buildRustPackage
  {
    pname = cargoToml.package.name;
    version = cargoToml.package.version;
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

    # For the use with rust-analyzer
    RUST_SRC_PATH = "${rustPlatform.rustLibSrc}";
  }
