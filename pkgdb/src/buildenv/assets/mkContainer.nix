# A wrapper around dockerTools.streamLayeredImage that
# composes a storePath to an environment with a shell and core utils
{
  # the (bundled) nixpkgs flake
  nixpkgsFlake,
  # the path to the environment that was built previously
  environmentOutPath,
  # the system to build for
  system,
  containerSystem,
}: let
  environment = builtins.storePath environmentOutPath;
  pkgs = nixpkgsFlake.legacyPackages.${system};
  containerPkgs = nixpkgsFlake.legacyPackages.${containerSystem};
  lib = pkgs.lib;
  lowPriority = pkg: pkg.overrideAttrs (old: old // {meta = (old.meta or {}) // {priority = 10000;};});

  buildLayeredImageArgs = {
    name = "flox-env-container";
    # symlinkJoin fails when drv contains a symlinked bin directory, so wrap in an additional buildEnv
    contents = pkgs.buildEnv {
      name = "contents";
      paths = [
        environment
        (lowPriority containerPkgs.bashInteractive) # for a usable shell
        (lowPriority containerPkgs.coreutils) # for just the basic utils
      ];
    };
    config = {};
  };
in
  pkgs.dockerTools.streamLayeredImage buildLayeredImageArgs
