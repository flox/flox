{
  inputs,
  nixpkgs,
  lib,
}: let
  # temporary, until commitizen 2.41.1 is avalable in nixpkgss
  commitizen = nixpkgs.commitizen.overridePythonAttrs (old: {
    doCheck = false;
    src = inputs.commitizen-src;
    meta = (old.meta or {}) // {mainProgram = "cz";};
  });

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
      inherit commitizen;
      inherit (nixpkgs) cargo clippy rustfmt alejandra;
    };
  })
  // {passthru = {inherit commitizen rustfmt;};}
