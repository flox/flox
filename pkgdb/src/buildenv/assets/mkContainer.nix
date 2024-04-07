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
    config = {
      # * run -it # [interactive, no args]
      #   -> runs <Entrypoint> <Cmd>
      #   -> bash -c -i bash --rcfile <activate>
      #   (skip activation for the first bash and runs default rcfiles)
      #
      # * run cmd... # [non-interactive, with arguments]
      #   -> BASH_ENV=<activate> bash -c cmd

      # * follow convention of sh -c being container entrypoint
      Entrypoint = ["${containerPkgs.bashInteractive}/bin/bash" "-c"];

      Env = lib.mapAttrsToList (name: value: "${name}=${value}") {
        "FLOX_ENV" = environment;
        "FLOX_PROMPT_ENVIRONMENTS" = "floxenv";
        "FLOX_PROMPT_COLOR_1" = "99";
        "FLOX_PROMPT_COLOR_2" = "141";
        "_FLOX_ACTIVE_ENVIRONMENTS" = "[]";
        "FLOX_SOURCED_FROM_SHELL_RC" = "1"; # don't source from shell rc (again)
        "BASH_ENV" = "${environment}/activate";
      };

      # source original .bashrc, then start another shell that runs activation
      Cmd = ["-i" "${containerPkgs.bashInteractive}/bin/bash -c ${environment}/activate"];
    };
  };
in
  pkgs.dockerTools.streamLayeredImage buildLayeredImageArgs
