{
  context,
  namespace,
  system,
  config,
  lib,
  ...
}: let
  floxpkgs = context.inputs.flox-floxpkgs;
  pkgs = context.nixpkgs;
in {
  options.container = with lib; {
    name = mkOption {
      description = mdDoc ''The name of the resulting image.'';
      type = types.str;
      default = builtins.elemAt namespace (builtins.length namespace - 1);
    };

    tag = mkOption {
      description = lib.mdDoc ''Tag of the generated image.'';
      type = types.str;
      default = "latest";
    };

    # fromImage = mkOption {
    #   description = lib.mdDoc ''The repository tarball containing the base image. It must be a valid Docker image, such as one exported by `docker save`.'';
    #   default = null;
    # };

    # contents = mkOption {
    #   description = lib.mdDoc ''Top-level paths in the container. Either a single derivation, or a list of derivations.'';
    #   default = [];
    # };

    # architecture = mkOption {
    #   description =
    #     lib.mdDoc ''
    #       used to specify the image architecture, this is useful for multi-architecture builds that don't need cross compiling. If not specified it will default to `hostPlatform`.'';
    #   default = {};
    # };

    config = mkOption {
      description = lib.mdDoc ''
        Run-time configuration of the container. A full list of the options
        available is in the
        [Docker Image Specification v1.2.0](https://github.com/moby/moby/blob/master/image/spec/v1.2.md#image-json-field-descriptions).
        Note that `config.env` is not supported (use `environmentVariables`
        instead).

        If `config.entrypoint` is not specified, flox activation will be
        performed in a bash shell.
      '';
      type = types.anything;
      default = {};
    };

    created = mkOption {
      description = lib.mdDoc ''Date and time the layers were created.'';
      type = types.str;
      default = "1970-01-01T00:00:02Z";
    };

    maxLayers = mkOption {
      description = lib.mdDoc ''Maximum number of layers to create. At most 125'';
      type = types.int;
      default = 100;
    };

    extraCommands = mkOption {
      description = lib.mdDoc ''
        Shell commands to run while building the final layer when the
        environment is transformed into a container. The commands do not have
        access to most of the layer contents. Changes to this layer are "on top"
        of all the other layers, so can create additional directories and files.
      '';
      type = types.lines;
      default = "";
    };

    # fakeRootCommands = mkOption {
    #   description = lib.mdDoc ''
    #     Shell commands to run while creating the archive for the final layer in
    #     a fakeroot environment. Unlike `extraCommands`, you can run `chown` to
    #     change the owners of the files in the archive, changing fakeroot's state
    #     instead of the real filesystem. The latter would require privileges that
    #     the build user does not have. Static binaries do not interact with the
    #     fakeroot environment. By default all files in the archive will be owned
    #     by root.
    #   '';
    #   default = null;
    # };

    # enableFakechroot = mkOption {
    #   description = lib.mdDoc ''
    #     Whether to run in `fakeRootCommands` in `fakechroot`, making programs
    #     behave as though `/` is the root of the image being created, while files
    #     in the Nix store are available as usual. This allows scripts that
    #     perform installation in `/` to work as expected. Considering that
    #     `fakechroot` is implemented via the same mechanism as `fakeroot`, the
    #     same caveats apply.
    #   '';
    #   default = false;
    # };
  };
  config = {
    passthru.buildLayeredImageArgs =
      lib.recursiveUpdate config.container
      # run flox activation only when entrypoint is not set
      # override config.Env no matter what
      (
        if config.container.config.Entrypoint or null == null
        then {
          config =
            {
              # * run -it -> bash -c bash (skip activation for the first bash as
              #   described below)
              # * run cmd -> bash -c cmd
              # * follow convention of sh -c being container entrypoint
              Entrypoint = ["${pkgs.bashInteractive}/bin/bash" "-c"];
              # this will lead to double activation if someone runs bash
              # non-interactively
              Env = ["BASH_ENV=${config.toplevel.outPath}/activate"];
            }
            // lib.optionalAttrs (config.container.config.Cmd
              or null
              == null) {
              # use -i to make entrypoint's bash non-interactive, so it skips
              # activation, but then activate with --rcfile. This sets aliases
              # correctly.
              Cmd = ["-i" "${pkgs.bashInteractive}/bin/bash --rcfile ${config.toplevel.outPath}/activate"];
            };
        }
        else {
          config = {
            Env =
              if builtins.isList config.environmentVariables
              then throw "ordered environment variables are not supported in containers when entrypoint is specified"
              else (lib.mapAttrsToList (n: v: ''${n}=${v}'') config.environmentVariables);
          };
        }
      );
    passthru.streamLayeredImage = floxpkgs.lib.mkContainer config.toplevel;
  };
}
