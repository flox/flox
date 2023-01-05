{inputs}:
inputs.shellHooks.lib.run {
  src = ../..;
  hooks = {
    alejandra.enable = true;
    rustfmt.enable = true;
    clippy.enable = true;
  };
  settings.clippy.denyWarnings = true;
}
