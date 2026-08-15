{
  config,
  pkgs,
  lib,
  ...
}:

let
  inherit (config.programs.flox) package;
  inherit (lib)
    escapeShellArgs
    filterAttrs
    mapAttrs
    mapAttrs'
    mapAttrsToList
    mkEnableOption
    mkIf
    mkMerge
    mkOption
    nameValuePair
    optionalAttrs
    optionals
    types
    ;

  cfg = config.services.flox;

  # Options common to both Flox module types.
  common = import ./common.nix { inherit lib; };

  # Function to calculate the working directory for a service.
  workingDirectory = name: "${cfg.stateDir}/${name}";

  floxActivationModule = {
    options = common.floxServiceOpts // {
      environment = mkOption {
        type = types.str;
        example = "flox/default";
        description = "The Flox environment to run the service from.";
      };
      user = mkOption {
        type = types.nullOr types.str;
        default = null;
        example = "root";
        description = ''
          The user with which to run the service.
          When null, a `flox-<name>` system user is created for the
          service.
        '';
      };
      group = mkOption {
        type = types.nullOr types.str;
        default = null;
        example = "root";
        description = ''
          The primary group membership for the service invocation.
          Must be set when `user` is set.
        '';
      };
      description = mkOption {
        type = types.nullOr types.str;
        default = null;
        example = "Foobar Web Server";
        description = "The systemd description for the service.";
      };
    };
  };

  serviceUser = name: aCfg: if aCfg.user != null then aCfg.user else "flox-${name}";
  serviceGroup = name: aCfg: if aCfg.user != null then aCfg.group else "flox-${name}";

  # The main unit's start command: activate the environment with its
  # services, and keep following their logs as the unit's foreground
  # process. The FloxHub token, if any, is provided as a systemd credential
  # and travels to flox only through the environment.
  startScript =
    name: aCfg:
    let
      activateCmd = escapeShellArgs (
        [ "${package}/bin/flox" ]
        ++ aCfg.extraFloxArgs
        ++ [
          "activate"
          "--dir"
          (workingDirectory name)
        ]
        ++ optionals aCfg.trustEnvironment [ "--trust" ]
        ++ aCfg.extraFloxActivateArgs
        ++ [ "--start-services" ]
      );
      logsCmd = escapeShellArgs (
        [ "${package}/bin/flox" ]
        ++ aCfg.extraFloxArgs
        ++ [
          "services"
          "logs"
          "--follow"
        ]
      );
    in
    pkgs.writeShellScript "flox-activate-${name}" ''
      set -euo pipefail
      if [ -n "''${CREDENTIALS_DIRECTORY:-}" ] && [ -e "$CREDENTIALS_DIRECTORY/floxhub_token" ]; then
        FLOX_FLOXHUB_TOKEN="$(cat "$CREDENTIALS_DIRECTORY/floxhub_token")"
        export FLOX_FLOXHUB_TOKEN
      fi
      # By the time systemd runs ExecStart the previous instance's cgroup is
      # gone, so any process-compose socket in this service's private cache
      # is stale. Left in place it prevents services from starting again
      # after an unclean shutdown.
      rm -f ${workingDirectory name}/.cache/flox/run/*.sock
      exec ${activateCmd} -- ${logsCmd}
    '';

in
{
  options = {
    services.flox = common.floxModuleOpts // {
      enable = mkEnableOption "running systemd services from Flox environments";
      activations = mkOption {
        type = types.attrsOf (types.submodule floxActivationModule);
        default = { };
        description = ''
          Flox environments to activate as systemd services.
          Each activation runs `flox activate --start-services` and
          delegates process management to the Flox services subsystem.
        '';
      };
    };
  };

  config = mkMerge [
    {
      assertions = [
        {
          assertion = cfg.activations == { } || cfg.enable;
          message = "services.flox.activations is set but services.flox.enable is false. Set services.flox.enable = true to run the configured activations.";
        }
      ]
      ++ mapAttrsToList (name: aCfg: {
        assertion = aCfg.user == null || aCfg.group != null;
        message = "services.flox.activations.${name}.group must be set when services.flox.activations.${name}.user is set.";
      }) cfg.activations;
    }

    (mkIf cfg.enable {
      # Create an account and group for each service that does not specify
      # its own. The pull script creates and owns the working directory,
      # which doubles as the service's home.
      users.users = mapAttrs' (
        name: aCfg:
        nameValuePair "flox-${name}" {
          isSystemUser = true;
          useDefaultShell = true;
          group = "flox-${name}";
          home = workingDirectory name;
        }
      ) (filterAttrs (_: aCfg: aCfg.user == null) cfg.activations);
      users.groups = mapAttrs' (name: _: nameValuePair "flox-${name}" { }) (
        filterAttrs (_: aCfg: aCfg.user == null) cfg.activations
      );

      # Provisioning, scheduled pulls and restart-on-change are handled by
      # the flox-pull@/flox-autopull@ template units; see pull.nix.
      services.flox.pull.configs = mapAttrs (name: aCfg: {
        unit = "${name}.service";
        user = serviceUser name aCfg;
        group = serviceGroup name aCfg;
        workingDirectory = workingDirectory name;
        inherit (aCfg)
          environment
          extraFloxArgs
          extraFloxPullArgs
          pullAtServiceStart
          floxHubTokenFile
          ;
        autoPull = aCfg.autoPull.enable;
        autoPullDates = aCfg.autoPull.dates;
        autoRestart = aCfg.autoRestart.enable;
      }) cfg.activations;

      systemd.services = mapAttrs (name: aCfg: {
        description =
          if aCfg.description != null then
            aCfg.description
          else
            "Flox ${name} service running from ${aCfg.environment} environment";
        wants = [ "network-online.target" ];
        after = [
          "network-online.target"
          "flox-pull@${name}.service"
        ];
        # The pull unit provisions the environment on first start and
        # refreshes it thereafter (see pullAtServiceStart). A failed
        # refresh of an existing environment exits successfully, so this
        # hard dependency only propagates initial provisioning failures.
        requires = [ "flox-pull@${name}.service" ];
        wantedBy = [ "multi-user.target" ];
        serviceConfig = {
          User = serviceUser name aCfg;
          Group = serviceGroup name aCfg;
          WorkingDirectory = workingDirectory name;
          Environment = common.serviceEnvironment pkgs.runtimeShell (workingDirectory name) (
            serviceUser name aCfg
          );
          ExecStart = "${startScript name aCfg}";
          Restart = "on-failure";
        }
        // optionalAttrs (aCfg.floxHubTokenFile != null) {
          LoadCredential = [ "floxhub_token:${aCfg.floxHubTokenFile}" ];
        };
      }) cfg.activations;
    })
  ];
}
