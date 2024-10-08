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
  containerName ? "flox-env-container",
  containerTag ? null,
  containerCreated ? "now",
}: let
  environment = builtins.storePath environmentOutPath;
  pkgs = nixpkgsFlake.legacyPackages.${system};
  containerPkgs = nixpkgsFlake.legacyPackages.${containerSystem};
  lib = pkgs.lib;
  lowPriority = pkg: pkg.overrideAttrs (old: old // {meta = (old.meta or {}) // {priority = 10000;};});

  buildLayeredImageArgs = {
    name = containerName;
    tag = containerTag;
    created = containerCreated;
    # symlinkJoin fails when drv contains a symlinked bin directory, so wrap in an additional buildEnv
    contents = pkgs.buildEnv {
      name = "contents";
      paths = [
        environment
        (lowPriority containerPkgs.bashInteractive) # for a usable shell
        (lowPriority containerPkgs.coreutils) # for just the basic utils
      ];
    };
    # Activate script requires writable /tmp.
    extraCommands = ''
      mkdir -m 1777 tmp
    '';
    config = {
      # Use activate script as the [one] entrypoint capable of
      # detecting interactive vs. command activation modes.
      # Usage:
      #   podman run -it
      #     -> launches interactive shell with controlling terminal
      #   podman run -i <cmd>
      #     -> invokes interactive command
      #   podman run -i [SIC]
      #     -> launches crippled interactive shell with no controlling
      #        terminal .. kinda useless
      Entrypoint = ["${environment}/activate"];

      Env = lib.mapAttrsToList (name: value: "${name}=${value}") {
        "FLOX_ENV" = environment;
        "FLOX_PROMPT_ENVIRONMENTS" = "floxenv";
        "FLOX_PROMPT_COLOR_1" = "99";
        "FLOX_PROMPT_COLOR_2" = "141";
        "_FLOX_ACTIVE_ENVIRONMENTS" = "[]";
        "FLOX_SOURCED_FROM_SHELL_RC" = "1"; # don't source from shell rc (again)
        "_FLOX_FORCE_INTERACTIVE" = "1"; # Required when running podman without "-t"
        "FLOX_SHELL" = "${containerPkgs.bashInteractive}/bin/bash";
      };
    };
  };
in
  pkgs.dockerTools.streamLayeredImage buildLayeredImageArgs
