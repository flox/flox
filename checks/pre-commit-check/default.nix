{
  system,
  shellHooks,
}:
shellHooks.lib.${system}.run {
  src = builtins.path {path = ../..;};
  hooks = {
    alejandra.enable = true;
    rustfmt.enable = true;
    commitizen.enable = true;
  };
}
