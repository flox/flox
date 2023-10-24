{
  shellHooks,
  rustfmt,
  cargo,
  commitizen,
  clippy,
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
      clippy.enable = true;
      commitizen.enable = true;
    };
    settings.clippy.denyWarnings = true;
    tools = {
      inherit cargo commitizen clippy rustfmt alejandra;
    };
  })
  // {passthru = {inherit rustfmt;};}
