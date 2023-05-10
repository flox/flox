{
  lib,
  self,
  ...
}: {injectedArgs ? {}}: {context, ...}: let
  floxEnvDir = context.self.outPath;
  floxNixPath = "${floxEnvDir}/flox.nix";
  catalogPath = "${floxEnvDir}/catalog.json";
  namespace = ["default"];
  result = lib.genAttrs context.systems (system: {
    default = lib.recursiveUpdate (self.lib.mkFloxEnv {
      inherit system namespace;
      context = context.context' system;
      modules = [floxNixPath] ++ lib.optional (builtins.pathExists catalogPath) {inherit catalogPath;};
    }) {meta.position = builtins.unsafeDiscardStringContext floxNixPath;};
  });
in {
  floxEnvs =
    if builtins.pathExists floxNixPath
    then result
    else {};
  devShells =
    if builtins.pathExists floxNixPath
    then result
    else {};
}
