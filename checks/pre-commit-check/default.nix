{
  inputs,
  nixpkgs,
  lib,
}: let
  # temporary, until https://github.com/commitizen-tools/commitizen/pull/644 is merged
  commitizen = nixpkgs.commitizen.overridePythonAttrs (_: {
    doCheck = false;
    src = builtins.fetchGit {
      url = "https://github.com/skoef/commitizen/";
      ref = "add-hooks-for-bump-command";
      rev = "103def9d6c290afe1b0daca359581cefadce11ce";
    };
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
