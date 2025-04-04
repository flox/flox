pkgsContext:
{
  config,
  options,
  pkgs,
  lib,
  system,
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

  # Module for installing and configuring Flox system-wide.
  floxProgramsModule =
    { ... }:
    {
      options.programs.flox = {
        enable = mkEnableOption "Flox CLI - Harness the power of Nix";
        package = mkOption {
          type = types.package;
          description = "Flox CLI package";
          default = pkgsContext.${system}.flox;
          defaultText = literalExpression "pkgs.flox";
          example = literalExpression "pkgs.flox";
        };
      };
      # Flox system-wide configuration
      config = mkMerge [
        {
          nix.settings = {
            trusted-public-keys = [
              "flox-cache-public-1:7F4OyH7ZCnFhcze3fJdfyXYLQw/aV7GEed86nQ7IsOs="
            ];
            substituters = [
              "https://cache.flox.dev"
            ];
          };
        }
        (mkIf programsCfg.enable {
          environment.systemPackages = [ programsCfg.package ];
        })
      ];
    };

  /*
    The following submodules offer two ways of configuring systemd
    services to run from Flox environments:

    1. floxServiceModule: configures systemd to activate environments
       with `flox activate --start-services`, delegating all process
       management thereafter to the Flox subsystem.

       Enabled with:
         services.flox.activations = {
           foo = {
             environment = "flox/demo";
             dynamicUser = true;
           };
         };

    2. floxOverridesModule: leverages existing NixOS modules for overriding
       the `ExecStart` option to run within the activated Flox environment.

       Enabled with:
         systemd.services.echoip.flox = {
           environment = "flox/echoip";
           trustEnvironment = true;
           autoPull.enable = true;
           execStart = "echoip -l 127.0.0.1:8080 -H X-Real-IP";
         };

    While the first of these presents the easiest/most intuitive interface
    from a Flox perspective, the second makes it possible to leverage the
    rich capabilities of the NixOS module subsystem, as well as the hundreds
    of existing NixOS modules maintained by the Nix community.
  */

  # Options common to both Flox module types.
  common = import ./common.nix { inherit lib; };

  floxActivationModule =
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
            "";
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

    in
    {
      options = common.floxModuleOpts // {
        description = mkOption {
          type = types.nullOr types.str;
          default = null;
          example = "Foobar Web Server";
          description = mdDoc "The systemd description for the service";
        };
      };
    };

  floxServicesModule =
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
            description = "FIX ME";
          };
          group = mkOption {
            type = types.str;
            default = "flox";
            description = "FIX ME";
          };
          floxHubTokenFile = mkOption {
            type = types.nullOr types.path;
            default = null;
            example = "/run/secrets/floxhub/secret.token";
            description = mdDoc "Full path to the FloxHub token file";
          };
          activations = mkOption {
            type = types.attrsOf (types.submodule floxActivationModule);
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
              flox = {
                group = serviceCfg.group;
                isSystemUser = true;
              };
            };
            users.groups = {
              flox = { };
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
                    services."${name}" =
                      let
                        floxActivateWithArgs = escapeShellArgs (
                          [
                            "${programsCfg.package}/bin/flox"
                            "activate"
                            "--remote"
                            activationCfg.environment
                          ]
                          ++ lib.optionals activationCfg.trustEnvironment [ "--trust" ]
                          ++ activationCfg.extraFloxActivateArgs
                        );
                      in
                      {
                        description =
                          if (activationCfg.description != null) then
                            activationCfg.description
                          else
                            "Flox ${name} service running from ${activationCfg.environment} environment";
                        wants = [ "network-online.target" ];
                        after = [ "network-online.target" ];
                        wantedBy = [ "multi-user.target" ];

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
                            ExecStart = "${floxActivateWithArgs} --start-services";
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

in
{
  imports = [
    floxProgramsModule
    floxServicesModule
    ./overrides.nix
    ./autopull.nix
  ];
}
