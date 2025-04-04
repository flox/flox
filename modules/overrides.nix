{
  config,
  options,
  pkgs,
  lib,
  name,
  utils,
  ...
}:
let
  inherit (utils.systemdUtils.lib) makeJobScript;
  inherit (lib)
    escapeShellArgs
    literalExpression
    mkBefore
    mdDoc
    mkDefault
    mkEnableOption
    mkForce
    mkIf
    mkMerge
    mkOption
    types
    ;

  programsCfg = config.programs.flox;

  # Options common to both Flox module types.
  common = import ./common.nix { inherit lib; };

  floxOverridesSubmodule =
    {
      options,
      config,
      name,
      ...
    }:
    let

      jobScripts = makeJobScript {
        name = "${name}-start";
        text =
          if (config.flox.script != "") then
            config.flox.script
          else if (config.script != "") then
            config.script
          else
            "";
        inherit (config) enableStrictShellChecks;
      };
      # Prefer config.flox.execStart over config{,.flox}.script.
      scriptAndArgs =
        if (config.flox.execStart != "") then
          config.flox.execStart
        else if (jobScripts != "") then
          "${jobScripts} ${config.scriptArgs}"
        else
          null;

      # We need a wrapper to detect and set things that are hard or impossible
      # to do at the Nix expression level. For example, services which set their
      # DynamicUser=true will not have a home directory, so will require certain
      # variables to be set.
      floxWrapper = pkgs.writeScript "flox-wrapper" ''
        #! ${pkgs.runtimeShell} -eu
        if [ -z "''${XDG_CACHE_HOME:-}" -o \
             -z "''${XDG_CONFIG_HOME:-}" -o \
             -z "''${XDG_DATA_HOME:-}" -o \
           ! -w "''${XDG_CACHE_HOME:-}" -o \
           ! -w "''${XDG_CONFIG_HOME:-}" -o \
           ! -w "''${XDG_DATA_HOME:-}" ]; then
          export XDG_CACHE_HOME=/tmp/.cache
          export XDG_DATA_HOME=/tmp/.local/share
          export XDG_CONFIG_HOME=/tmp/.config
        fi
        exec -a ${programsCfg.package}/bin/flox ${programsCfg.package}/bin/flox "$@"
      '';

      floxWrapperWithArgs = escapeShellArgs ([ floxWrapper ] ++ config.flox.extraFloxArgs);

      floxActivateWithArgs = escapeShellArgs (
        [
          floxWrapperWithArgs
          "activate"
          "-r"
          config.flox.environment
        ]
        ++ lib.optionals config.flox.trustEnvironment [ "--trust" ]
        ++ config.flox.extraFloxActivateArgs
      );

      floxAuthLoginWithArgs = escapeShellArgs [
        floxWrapperWithArgs
        "auth"
        "login"
        "with-token"
        "<"
        config.flox.floxHubTokenFile
      ];

      floxPullWithArgs = escapeShellArgs (
        [
          floxWrapperWithArgs
          "pull"
          "-r"
          config.flox.environment
        ]
        ++ config.flox.extraFloxPullArgs
      );

    in
    {
      options = {
        flox = common.floxModuleOpts // {
          execStart = mkOption {
            type = types.str;
            default = "";
            description = mdDoc "The command to override the unit's ExecStart with";
          };
          script = mkOption {
            type = types.str;
            default = "";
            description = mdDoc "A script to entirely replace the unit's script";
          };

          # Enable floxOverridesModule submodule for overriding the execStart attribute
          # of any given systemd services to run within a Flox environment.
          systemd.services = mkOption {
            type = types.attrsOf (types.submodule floxOverridesModule);
          };
        };
      };

      config = mkIf (config.flox.environment != null) {
        serviceConfig = mkMerge [
          # Default service config
          {
            Environment = [
              # FIXME: add flag for disabling metrics
              "FLOX_DISABLE_METRICS=true"
            ];
          }

          /*
             Would love to be able to refer to config.serviceConfig.*
             in the config but cannot on account of infinite recursion.

            (mkIf (config.serviceConfig.DynamicUser) {
              # breaks w/ infinite recursion
              Environment = [
                "XDG_CACHE_HOME=/tmp/.cache"
                "XDG_DATA_HOME=/tmp/.local/share"
                "XDG_CONFIG_HOME=/tmp/.config"
              ];
            })

            (mkIf (scriptAndArgs == null) {
              # Prepend Flox activation to existing ExecStart line
              ExecStart = mkForce "${floxActivateWithArgs} -- ${config.serviceConfig.ExecStart}";
            })
          */

          (mkIf (scriptAndArgs != null) {
            # Completely override the ExecStart config
            ExecStart = mkForce "${floxActivateWithArgs} -- ${scriptAndArgs}";
          })

          # Workaround so the service can pull the environment from private repositories
          (mkIf (config.flox.floxHubTokenFile != null) {
            # TODO: update `flox auth login` to accept `--with-token` and
            #       read from STDIN (like `gh auth`)
            ExecStartPre = [ "/bin/sh -c '${floxAuthLoginWithArgs}'" ];
          })
          # Pull the environment at service start
          (mkIf (config.flox.pullAtServiceStart) {
            ExecStartPre = [ floxPullWithArgs ];
          })
        ];
      };
    };

  floxOverridesModule = {
    options = {
      # Enable floxOverridesSubmodule submodule for overriding the execStart attribute of any
      # given systemd services to run within a Flox environment.
      systemd.services = mkOption {
        type = types.attrsOf (types.submodule floxOverridesSubmodule);
      };
    };

  };

in
floxOverridesModule
