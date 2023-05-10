{
  lib,
  self,
  ...
}: {
  sourceType,
  injectedArgs ? {},
  dir ? sourceType,
}: let
  materialize = lib.capacitor.capacitate.materialize;
in
  {context, ...}: let
    floxEnvsMapper = context: {
      namespace,
      system,
      outerPath,
      ...
    }: let
      floxEnvDir = let
        # if dir is a string, assume that it's in the root flake. If it's a path, leave as is (which
        # supports flakes in sub-directories)
        fullDir =
          if builtins.isPath dir
          then [dir]
          else [context.self.outPath dir];
      in
        builtins.concatStringsSep "/" (fullDir ++ outerPath);
      floxNixPath = "${floxEnvDir}/flox.nix";
      catalogPath = "${floxEnvDir}/catalog.json";
    in {
      use = builtins.pathExists floxNixPath;
      # for now just treat flox.nix as a module, although at some point we might want to do something like
      # context.auto.callPackageWith injectedArgs floxNixPath {};
      value =
        lib.recursiveUpdate
        (self.lib.mkFloxEnv {
          inherit system namespace;
          context = context.context' system;
          modules = [floxNixPath] ++ lib.optional (builtins.pathExists catalogPath) {inherit catalogPath;};
        }) {meta.position = builtins.unsafeDiscardStringContext floxNixPath;};
      path = [system] ++ namespace;
    };
    result = materialize (floxEnvsMapper context) (context.closures sourceType);
  in {
    floxEnvs = result;
    devShells = result;
  }
