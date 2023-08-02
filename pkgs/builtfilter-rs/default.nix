{
  inputs,
  pkgs,
  lib,
  self,
  ...
}: let
  cargoToml = lib.importTOML "${self}/builtfilter-rs/Cargo.toml";
in
  pkgs.rustPlatform.buildRustPackage
  rec
  {
    pname = cargoToml.package.name;
    version = cargoToml.package.version;
    cargoLock = {
      lockFile = src + "/Cargo.lock";
      allowBuiltinFetchGit = true;
    };
    src = self + "/builtfilter-rs";
    nativeBuildInputs = [pkgs.pkg-config];
    buildInputs =
      [pkgs.openssl]
      ++ lib.optional pkgs.stdenv.isDarwin [
        pkgs.libiconv
        pkgs.darwin.apple_sdk.frameworks.Security
      ];
  }
