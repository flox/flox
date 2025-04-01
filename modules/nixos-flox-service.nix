pkgsContext:
{
  config,
  options,
  pkgs,
  lib,
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
  serviceCfg = config.services.flox;

  floxActivationMod =
    {
      options,
      name,
      ...
    }:
    let
      activationCfg = serviceCfg.activations.${name};

      jobScripts = makeJobScript {
        name = "${name}-start";
        text =
          if (config.flox.script != "") then
            config.flox.script
          else if (config.script != "") then
            config.script
          else
            "WHATEVER";
        inherit (config) enableStrictShellChecks;
      };
      # Prefer config.flox.execStart over config{,.flox}.script.
      scriptAndArgs =
        if (config.flox.execStart != "") then
          config.flox.execStart
        else if (config.flox.script != "" || config.script != "") then
          "${jobScripts} ${config.scriptArgs}"
        else
          null;

      # # We need a wrapper to detect and set things that are hard or impossible
      # # to do at the Nix expression level. For example, services which set their
      # # DynamicUser=true will not have a home directory, so will require certain
      # # variables to be set.
      # floxWrapper = pkgs.writeScript "flox-wrapper" ''
      #   #! ${pkgs.runtimeShell} -eu
      #   if [ -z "''${XDG_CACHE_HOME:-}" -o \
      #        -z "''${XDG_CONFIG_HOME:-}" -o \
      #        -z "''${XDG_DATA_HOME:-}" -o \
      #      ! -w "''${XDG_CACHE_HOME:-}" -o \
      #      ! -w "''${XDG_CONFIG_HOME:-}" -o \
      #      ! -w "''${XDG_DATA_HOME:-}" ]; then
      #     export XDG_CACHE_HOME=/tmp/.cache
      #     export XDG_DATA_HOME=/tmp/.local/share
      #     export XDG_CONFIG_HOME=/tmp/.config
      #   fi
      #   exec -a ${cfg.package}/bin/flox ${cfg.package}/bin/flox "$@"
      # '';

      # floxWrapperWithArgs = escapeShellArgs ([ floxWrapper ] ++ config.flox.extraFloxArgs);

      # floxActivateWithArgs = escapeShellArgs (
      #   [
      #     floxWrapperWithArgs
      #     "activate"
      #     "-r"
      #     config.flox.environment
      #   ]
      #   ++ lib.optionals config.flox.trustEnvironment [ "--trust" ]
      #   ++ config.flox.extraFloxActivateArgs
      # );

      floxAuthLoginWithToken =
        tokenFilePath:
        escapeShellArgs [
          config.programs.flox.package
          "config"
          "--set"
          "floxhub_token"
          "$(${pkgs.coreutils}/bin/cat ${tokenFilePath})"
          # floxWrapperWithArgs
          # "auth"
          # "login"
          # "with-token"
          # "<"
          # config.flox.floxHubTokenFile
        ];

      # floxPullWithArgs = escapeShellArgs (
      #   [
      #     floxWrapperWithArgs
      #     "pull"
      #     "-r"
      #     config.flox.environment
      #   ]
      #   ++ config.flox.extraFloxPullArgs
      # );

    in
    {
      options = {
        environment = mkOption {
          type = types.nullOr types.str;
          default = null;
          example = "flox/default";
          description = mdDoc "The Flox environment to use for the service";
        };
        trustEnvironment = mkOption {
          type = types.bool;
          default = false;
          description = mdDoc "Whether to trust the environment using invocation option";
        };
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
        extraFloxArgs = mkOption {
          type = types.listOf types.str;
          default = [ ];
          example = "-v -v";
          description = mdDoc "Additional arguments to pass to `flox`";
        };
        extraFloxActivateArgs = mkOption {
          type = types.listOf types.str;
          default = [ ];
          example = "--mode dev";
          description = mdDoc "Additional arguments to pass to `flox activate`";
        };
        extraFloxPullArgs = mkOption {
          type = types.listOf types.str;
          default = [ ];
          example = [ "--force" ];
          description = mdDoc "Additional arguments to pass to `flox pull`";
        };
        pullAtServiceStart = mkOption {
          type = types.bool;
          default = false;
          description = mdDoc "Whether to pull the Flox environment at service start";
        };
        floxServiceManager = mkOption {
          type = types.bool;
          default = false;
          description = mdDoc "Whether to use the internal Flox service management";
        };
        autoPull.enable = mkOption {
          type = types.bool;
          default = false;
          description = mdDoc "Whether to automatically pull the Flox environment";
        };
        autoPull.dates = mkOption {
          type = types.str;
          default = "00:00";
          example = "daily";
          description = lib.mdDoc ''
            How often or when upgrade occurs.

            The format is described in
            {manpage}`systemd.time(7)`.
          '';
        };
        autoRestart.enable = mkOption {
          type = types.bool;
          default = false;
          description = mdDoc "Whether to automatically restart the service when the Flox environment changes";
        };
      };

      #     serviceConfig = mkMerge [
      #       # Default service config
      #       {
      #         Environment = [
      #           # FIXME: add flag for disabling metrics
      #           "FLOX_DISABLE_METRICS=true"
      #         ];
      #       }

      #       (mkIf (config.serviceConfig.DynamicUser) {
      #         # breaks w/ infinite recursion
      #         Environment = [
      #           "XDG_CACHE_HOME=/tmp/.cache"
      #           "XDG_DATA_HOME=/tmp/.local/share"
      #           "XDG_CONFIG_HOME=/tmp/.config"
      #         ];
      #       })

      #       # (mkIf (scriptAndArgs != null) {
      #       #   # Completely override the ExecStart config
      #       #   ExecStart = mkForce "${floxActivateWithArgs} -- ${scriptAndArgs}";
      #       # })

      #       (mkIf (scriptAndArgs == null) {
      #         # Prepend Flox activation to existing ExecStart line
      #         ExecStart = mkForce "${floxActivateWithArgs} -- ${config.serviceConfig.ExecStart}"; # breaks w/ infinite recursion
      #       })

      #       # config.serviceConfig.ExecStart;

      #       # Workaround so the service can pull the environment from private repositories
      #       (mkIf (config.flox.floxHubTokenFile != null) {
      #         # TODO: update `flox auth login` to accept `--with-token` and
      #         #       read from STDIN (like `gh auth`)
      #         ExecStartPre = [ "/bin/sh -c '${floxAuthLoginWithArgs}'" ];
      #       })
      #       # Pull the environment at service start
      #       (mkIf (config.flox.pullAtServiceStart) {
      #         ExecStartPre = [ floxPullWithArgs ];
      #       })
      #     ];
      #   };
      # };

    };

  floxServiceMod =
    { ... }:
    {

      imports = [ ];
      # Options for system-wide activations
      options = {
        services.flox = {
          enable = mkEnableOption "Flox CLI - Harness the power of Nix";
          user = mkOption {
            type = types.str;
            default = "flox";
          };
          floxHubTokenFile = mkOption {
            type = types.nullOr types.path;
            default = null;
            example = "/run/secrets/floxhub/secret.token";
            description = mdDoc "Full path to the FloxHub token file";
          };
          activations = mkOption {
            type = types.attrsOf (types.submodule floxActivationMod);
            default = { };
          };
        };
      };

      config =
        let
          enableFlox = {
            programs.flox.enable = true;
          };

          floxUser = mkIf (serviceCfg.user == options.services.flox.user.default) {
            users.users = {
              "flox" = {
                isSystemUser = true;
              };
            };
          };

          floxLoginUnit = mkIf (serviceCfg.floxHubTokenFile != null) {
            systemd.services."flox-login" = {
              serviceConfig = {
                User = serviceCfg.user;
                Type = "oneshot";
                ExecStart = escapeShellArgs [
                  "${config.programs.flox.package}/bin/flox"
                  "config"
                  "--set"
                  "floxhub_token"
                  "$(${pkgs.coreutils}/bin/cat ${serviceCfg.floxHubTokenFile})"
                ];
              };
            };
          };

          activationUnits = {
            systemd = mkMerge (
              lib.mapAttrsToList (
                name: activationCfg:
                mkMerge [
                  (mkIf activationCfg.autoPull.enable {
                    timers."flox-autoPull@${activationCfg.environment}" = {
                      timerConfig = {
                        User = serviceCfg.user;
                        RandomizedDelaySec = "15s";
                        OnCalendar = activationCfg.autoPull.dates;
                        Unit = "flox-autoPull@${activationCfg.environment}.service";
                      };
                    };

                    services."flox-autoPull@${activationCfg.environment}" = {
                      serviceConfig = {
                        User = serviceCfg.user;
                        Type = "oneshot";
                        ExecStart = "${programsCfg.package}/bin/flox pull --remote \"${activationCfg.environment}\"";
                      };
                    };
                  })

                  {
                    services."flox-activation@${name}" =
                      let
                        jobScripts = makeJobScript {
                          name = "${name}-start";
                          text =
                            if (activationCfg.script != "") then
                              activationCfg.script
                            else if (config.script != "") then
                              config.script
                            else
                              "WHATEVER";
                          inherit (config) enableStrictShellChecks;
                        };

                        # Prefer activationCfg.execStart over config{,.flox}.script.
                        scriptAndArgs =
                          if (activationCfg.execStart != "") then
                            activationCfg.execStart
                          else if (activationCfg.script != "") then
                            "${jobScripts} ${activationCfg.scriptArgs}"
                          else
                            # todo: encode in type
                            throw "must specify either a script or execStart command";

                        floxActivateWithArgs = escapeShellArgs (
                          [
                            programsCfg.package
                            "activate"
                            "--remote"
                            activationCfg.environment
                          ]
                          ++ lib.optionals activationCfg.trustEnvironment [ "--trust" ]
                          ++ activationCfg.extraFloxActivateArgs
                        );
                      in
                      {
                        serviceConfig = mkMerge [
                          # Default service config
                          {
                            Environment = [
                              # FIXME: add flag for disabling metrics
                              "FLOX_DISABLE_METRICS=true"
                            ];
                          }

                          # Set the ExecStart config
                          {
                            ExecStart = "${floxActivateWithArgs} -- ${scriptAndArgs}";
                          }

                          # Workaround so the service can pull the environment from private repositories
                          (mkIf (serviceCfg.floxHubTokenFile != null) {
                            After = [ "flox-login.service" ];
                            Requires = [ "flox-login.service" ];
                          })

                          # Pull the environment at service start
                          (mkIf (activationCfg.pullAtServiceStart) {
                            After = [ "flox-autoPull@${activationCfg.environment}.service" ];
                            Requires = [ "flox-autoPull@${activationCfg.environment}.service" ];
                          })
                        ];
                      };
                  }

                ]
              ) serviceCfg.activations
            );
          };

        in

        mkIf serviceCfg.enable (mkMerge [
          enableFlox
          floxUser
          floxLoginUnit
          activationUnits
        ]);

    };

  floxProgramsMod =
    { ... }:
    {
      options = {
        programs.flox = {
          enable = mkEnableOption "Flox CLI - Harness the power of Nix";
          package = mkOption {
            type = types.package;
            description = "Flox CLI package";
            default = pkgsContext.${pkgs.system}.flox;
            defaultText = literalExpression "pkgs.flox";
            example = literalExpression "pkgs.flox";
          };
        };
      };

      config = mkIf programsCfg.enable {
        nix.settings = {
          trusted-public-keys = [
            "flox-cache-public-1:7F4OyH7ZCnFhcze3fJdfyXYLQw/aV7GEed86nQ7IsOs="
          ];
          substituters = [
            "https://cache.flox.dev"
          ];
        };

      };

    };

in
{
  imports = [
    floxProgramsMod
    floxServiceMod
  ];
}
