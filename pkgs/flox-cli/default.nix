{
  self,
  lib,
  rustPlatform,
  hostPlatform,
  openssl,
  pkg-config,
  darwin,
  flox,
  nix,
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

    NIX_BIN = "${nix}/bin/nix";
    FLOX_SH = "${flox}/libexec/flox/flox";

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
