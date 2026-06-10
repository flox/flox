# Pattern: non-Python package (Rust / maturin) with no catalog argument at all.
# Exercises the case where there is no lambda pattern containing `catalogs`.
{
  buildPythonPackage,
  rustPlatform,
  openssl,
  pkg-config,
  sqlite,
}:

let
  src = ../../../..;

in
buildPythonPackage {
  pname = "native-ffi";
  inherit src;
  version =
    (builtins.fromTOML
      (builtins.readFile "${src}/native_ffi/Cargo.toml")).package.version;
  pyproject = true;

  cargoDeps = rustPlatform.importCargoLock {
    lockFile = "${src}/Cargo.lock";
  };

  nativeBuildInputs = [
    rustPlatform.cargoSetupHook
    rustPlatform.maturinBuildHook
    pkg-config
  ];

  buildInputs = [
    openssl
    sqlite
  ];

  maturinBuildFlags = [ "--manifest-path" "native_ffi/Cargo.toml" ];

  doCheck = false;
}
