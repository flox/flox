{
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/ab5fd150146dcfe41fda501134e6503932cc8dfd";
  outputs = {
    self,
    nixpkgs,
    ...
  }: let
    pkgs = nixpkgs.legacyPackages.aarch64-darwin;
  in {
    packages.aarch64-darwin.default = pkgs.stdenv.mkDerivation {
      pname = "dummy_package";
      version = "0.0.0";
      buildCommand = "mkdir -p \"$out\"";
      meta.broken = true;
    };
    packages.aarch64-linux.default = pkgs.stdenv.mkDerivation {
      pname = "dummy_package";
      version = "0.0.0";
      buildCommand = "mkdir -p \"$out\"";
      meta.broken = true;
    };
    packages.x86_64-darwin.default = pkgs.stdenv.mkDerivation {
      pname = "dummy_package";
      version = "0.0.0";
      buildCommand = "mkdir -p \"$out\"";
      meta.broken = true;
    };
    packages.x86_64-linux.default = pkgs.stdenv.mkDerivation {
      pname = "dummy_package";
      version = "0.0.0";
      buildCommand = "mkdir -p \"$out\"";
      meta.broken = true;
    };
  };
}
