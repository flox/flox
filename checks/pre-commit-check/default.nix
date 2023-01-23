{
  inputs,
  nixpkgs,
  lib,
}: let
  # temporary, until https://github.com/commitizen-tools/commitizen/pull/644 is merged
  commitizen = nixpkgs.commitizen.overridePythonAttrs (old: {
    doCheck = false;
    src = builtins.fetchGit {
      url = "https://github.com/skoef/commitizen/";
      ref = "add-hooks-for-bump-command";
      rev = "26b38beb8d507e4a4ee3c062639a96230c33dd92";
    };
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
    tools = {inherit commitizen;};
  })
  // {passthru = {inherit commitizen rustfmt;};}
