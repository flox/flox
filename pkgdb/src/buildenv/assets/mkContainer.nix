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
  pkgs = nixpkgsFlake.legacyPackages.${system};
  buildLayeredImageArgs = {
    name = "flox-env-container";
    # symlinkJoin fails when drv contains a symlinked bin directory, so wrap in an additional buildEnv
    contents = pkgs.buildEnv {
      name = "contents";
      paths = [
        environmentOutPath
        pkgs.bashInteractive # for a usable shell
        pkgs.coreutils # for just the basic utils
      ];
    };
    config = {};
  };
in
  pkgs.dockerTools.streamLayeredImage buildLayeredImageArgs
