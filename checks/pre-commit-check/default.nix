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
      rev = "d7c86c8244af0cb5be1ab2b3882d0af74a191352";
    };
  });
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
  // {passthru.commitizen = commitizen;}
