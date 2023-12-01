{
  shellHooks,
  rustfmt,
  cargo,
  commitizen,
  alejandra,
  system,
  ...
}: let
  shellHooksLib = builtins.getAttr system shellHooks.lib;
in
  (shellHooksLib.run {
    src = builtins.path {path = ../..;};
    hooks = {
      alejandra.enable = true;
      rustfmt.enable = true;
      commitizen.enable = true;
    };
    tools = {
      inherit cargo commitizen rustfmt alejandra;
    };
  })
  // {passthru = {inherit rustfmt;};}
