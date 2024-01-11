# A wrapper around dockerTools.streamLayeredImage that
# composes a storePath to an environment with a shell and core utils
{
  # the (bundled) nixpkgs flake
  nixpkgsFlake,
  # the path to the environment that was built previously
  environmentOutPath,
  # the system to build for
  system,
}: let
  environmentOutPath' = builtins.storePath environmentOutPath;
  pkgs = nixpkgsFlake.legacyPackages.${system};
  lowPriority = pkg: pkg.overrideAttrs (old: old // {meta = (old.meta or {}) // {priority = 10000;};});

  buildLayeredImageArgs = {
    name = "flox-env-container";
    # symlinkJoin fails when drv contains a symlinked bin directory, so wrap in an additional buildEnv
    contents = pkgs.buildEnv {
      name = "contents";
      paths = [
        environmentOutPath'
        (lowPriority pkgs.bashInteractive) # for a usable shell
        (lowPriority pkgs.coreutils) # for just the basic utils
      ];
    };
    config = {};
  };
in
  pkgs.dockerTools.streamLayeredImage buildLayeredImageArgs
