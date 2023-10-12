{
  pkgs,
  lib,
  ...
}: let
  src = ../../builtfilter-rs;
  cargoToml = lib.importTOML "${src}/Cargo.toml";
in
  pkgs.rustPlatform.buildRustPackage
  {
    pname = cargoToml.package.name;
    version = cargoToml.package.version;
    cargoLock = {
      lockFile = "${src}/Cargo.lock";
      allowBuiltinFetchGit = true;
    };
    src = src;
    nativeBuildInputs = [pkgs.pkg-config];
    buildInputs =
      [pkgs.openssl]
      ++ lib.optional pkgs.stdenv.isDarwin [
        pkgs.libiconv
        pkgs.darwin.apple_sdk.frameworks.Security
      ];
  }
