{
  inputs,
  nixpkgs,
  lib,
}: let
  rustfmt = nixpkgs.rustfmt.override {asNightly = true;};
in
  (inputs.shellHooks.lib.run {
    src = ../..;
    hooks = {
      alejandra.enable = true;
      rustfmt.enable = true;
      clippy.enable = true;
      commitizen.enable = true;
    };
    settings.clippy.denyWarnings = true;
    tools = {
      inherit (nixpkgs) cargo commitizen clippy rustfmt alejandra;
    };
  })
  // {passthru = {inherit rustfmt;};}
