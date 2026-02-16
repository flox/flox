{
  config,
  options,
  pkgs,
  lib,
  utils,
  ...
}:

let
  inherit (config.programs.flox) package;
  inherit (config.services.flox) activations stateDir workingDirectoryMode;
  inherit (lib)
    escapeShellArgs
    mdDoc
    mkIf
    mkMerge
    mkOption
    types
    ;

  # Options common to both Flox module types.
  common = import ./common.nix { inherit lib; };

  # Function to calculate the working directory for a service.
  workingDirectory = name: "${stateDir}/${name}";

  floxActivationModule =
    {
      options,
      name,
      ...
    }:
    let
      activationCfg = activations.${name};

    in
    {
      options = common.floxServiceOpts // {
        user = mkOption {
          type = types.nullOr types.str;
          default = null;
          example = "root";
          description = mdDoc "The user with which to run the service";
        };
        group = mkOption {
          type = types.nullOr types.str;
          default = null;
          example = "root";
          description = mdDoc "The primary group membership for the service invocation";
        };
        description = mkOption {
          type = types.nullOr types.str;
          default = null;
          example = "Foobar Web Server";
          description = mdDoc "The systemd description for the service";
        };
      };
    };

  floxServicesModule = {

    # Options for system-wide activations
    options = {
      services.flox = common.floxModuleOpts // {
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

        activationConfigs = {

          users = mkMerge (
            lib.mapAttrsToList (
              name: activationCfg:

              (mkIf (activationCfg.user == null) {
                # Create account and group for the service.
                users."flox-${name}" = {
                  isSystemUser = true;
                  useDefaultShell = true;
                  group = "flox-${name}";
                  home = workingDirectory name;
                  createHome = true;
                  homeMode = workingDirectoryMode;
                };
                groups."flox-${name}" = { };
              })
            ) activations
          );

          systemd = mkMerge (
            lib.mapAttrsToList (
              name: activationCfg:

              let
                WorkingDirectory = workingDirectory name;

                defaultUserGroupAttrs = {
                  User = "flox-${name}";
                  Group = "flox-${name}";
                };
                providedUserGroupAttrs =
                  if (activationCfg.user != null -> activationCfg.group != null) then
                    {
                      User = activationCfg.user;
                      Group = activationCfg.group;
                    }
                  else
                    throw "\nOption services.flox.activations.${name}.group is not set when services.flox.activations.${name}.user is specified.";
                userGroupAttrs =
                  if (activationCfg.user == null) then defaultUserGroupAttrs else providedUserGroupAttrs;

                Environment = [
                  "FLOX_DISABLE_METRICS=true"
                  "HOME=${WorkingDirectory}"
                  "SHELL=${pkgs.runtimeShell}"
                ];

                # Variables to customize flox invocations.
                floxWithArgs = [ "${package}/bin/flox" ] ++ activationCfg.extraFloxArgs;

                floxAuthLoginWithArgs =
                  escapeShellArgs (
                    floxWithArgs
                    ++ [
                      "config"
                      "--set"
                      "floxhub_token"
                    ]
                  )
                  + " \"$(cat ${activationCfg.floxHubTokenFile})\"";

                floxPullWithArgs = escapeShellArgs (
                  floxWithArgs
                  ++ [
                    "pull"
                    "--force"
                    activationCfg.environment
                  ]
                  ++ activationCfg.extraFloxPullArgs
                );

                floxActivateWithArgs = escapeShellArgs (
                  floxWithArgs
                  ++ [
                    "activate"
                    "--dir"
                    WorkingDirectory
                  ]
                  ++ lib.optionals activationCfg.trustEnvironment [ "--trust" ]
                  ++ activationCfg.extraFloxActivateArgs
                );

                commonServiceConfig = {
                  inherit Environment WorkingDirectory;
                } // userGroupAttrs;

              in
              mkMerge [

                # Create the working directory for the service.
                {
                  tmpfiles.rules = [
                    "d ${WorkingDirectory} ${workingDirectoryMode} ${userGroupAttrs.User} ${userGroupAttrs.Group} - -"
                  ];
                }

                # Create the flox-login@${name} service for logging into FloxHub.
                (mkIf (activationCfg.floxHubTokenFile != null) {
                  services."flox-login@${name}" = {
                    serviceConfig = commonServiceConfig // {
                      Type = "oneshot";
                      # N.B. must run this in a subshell to be able to `cat` the token file.
                      ExecStart = [ "${pkgs.runtimeShell} -c '${floxAuthLoginWithArgs}'" ];
                    };
                  };
                })

                # Create the flox-pull@${name} service for pulling updates.
                {
                  services."flox-pull@${name}" = mkMerge [
                    {
                      serviceConfig = commonServiceConfig // {
                        Type = "oneshot";
                        # N.B. must run this in a subshell to be able to `cat` the token file.
                        ExecStart = floxPullWithArgs;
                      };
                    }
                    (mkIf (activationCfg.floxHubTokenFile != null) {
                      unitConfig = {
                        # Workaround so the service can pull the environment from private repositories
                        After = [ "flox-login@${name}.service" ];
                        Requires = [ "flox-login@${name}.service" ];
                      };
                    })
                  ];
                }

                # Create the flox-autoPull@${name} timer for automatically
                # pulling and restarting services on a schedule.
                (mkIf activationCfg.autoPull.enable {
                  timers."flox-autoPull@${name}" = {
                    timerConfig = {
                      RandomizedDelaySec = "15s";
                      OnCalendar = activationCfg.autoPull.dates;
                      Unit = "flox-pull@${name}.service";
                    };
                  };
                })

                # Create the ${name} service for running the service.
                {
                  services."${name}" = {
                    description =
                      if (activationCfg.description != null) then
                        activationCfg.description
                      else
                        "Flox ${name} service running from ${activationCfg.environment} environment";
                    wants = [ "network-online.target" ];
                    after = [ "network-online.target" ];
                    wantedBy = [ "multi-user.target" ];
                    serviceConfig = commonServiceConfig // {
                      ExecStart =
                        "${floxActivateWithArgs} --start-services -- "
                        + "${escapeShellArgs floxWithArgs} services logs --follow";
                    };
                    unitConfig = mkMerge [
                      {
                        After = [ "flox-pull@${name}.service" ];
                      }
                      (mkIf (activationCfg.pullAtServiceStart) {
                        Requires = [ "flox-pull@${name}.service" ];
                      })
                    ];
                  };
                }

              ]
            ) activations
          );

        };

      in

      (mkMerge [
        enableFlox
        activationConfigs
      ]);

  };

in
floxServicesModule
