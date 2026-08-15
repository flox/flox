{
  config,
  pkgs,
  lib,
  utils,
  ...
}:

let
  inherit (config.programs.flox) package;
  inherit (config.services.flox) stateDir;
  inherit (utils.systemdUtils.lib) makeJobScript;
  inherit (lib)
    escapeShellArgs
    filterAttrs
    mapAttrs
    mapAttrsToList
    mkForce
    mkIf
    mkMerge
    mkOption
    optionals
    types
    ;

  # Options common to both Flox module types.
  common = import ./common.nix { inherit lib; };

  # Function to calculate the working directory for a service.
  workingDirectory = name: "${stateDir}/${name}";

  # Units that have been handed over to Flox. Only ever used to define
  # options outside of `systemd.services` (pull configs, assertions):
  # feeding it back into `systemd.services` would make the merge depend on
  # its own result.
  floxManagedServices = filterAttrs (_: svc: svc.flox.environment != null) config.systemd.services;

  floxOverridesSubmodule =
    {
      config,
      name,
      ...
    }:
    let
      fCfg = config.flox;
      WorkingDirectory = workingDirectory name;

      scriptText = if fCfg.script != "" then fCfg.script else config.script;
      jobScript = makeJobScript {
        name = "${name}-flox-start";
        text = scriptText;
        inherit (config) enableStrictShellChecks;
      };
      # Prefer flox.execStart, then flox.script, then the unit's own script.
      # The null case is rejected by an assertion in the top-level module;
      # the "false" placeholder only guards stray evaluations that bypass
      # the assertion machinery.
      mainCommand =
        if fCfg.execStart != "" then
          fCfg.execStart
        else if scriptText != "" then
          "${jobScript} ${config.scriptArgs}"
        else
          "false";

      activateCmd = escapeShellArgs (
        [ "${package}/bin/flox" ]
        ++ fCfg.extraFloxArgs
        ++ [
          "activate"
          "--dir"
          WorkingDirectory
        ]
        ++ optionals fCfg.trustEnvironment [ "--trust" ]
        ++ fCfg.extraFloxActivateArgs
      );

      # The FloxHub token, if any, is provided as a systemd credential and
      # travels to flox only through the environment.
      execStart = pkgs.writeShellScript "flox-exec-start-${name}" ''
        set -euo pipefail
        if [ -n "''${CREDENTIALS_DIRECTORY:-}" ] && [ -e "$CREDENTIALS_DIRECTORY/floxhub_token" ]; then
          FLOX_FLOXHUB_TOKEN="$(cat "$CREDENTIALS_DIRECTORY/floxhub_token")"
          export FLOX_FLOXHUB_TOKEN
        fi
        exec ${activateCmd} -- ${mainCommand}
      '';

    in
    {
      options = {
        flox = common.floxServiceOpts // {
          environment = mkOption {
            type = types.nullOr types.str;
            default = null;
            example = "flox/default";
            description = ''
              The Flox environment to run this unit from.
              When null (the default) the unit is left untouched.
            '';
          };
          execStart = mkOption {
            type = types.str;
            default = "";
            example = "echoip -l 127.0.0.1:8080 -H X-Real-IP";
            description = "The command to override the unit's ExecStart with.";
          };
          script = mkOption {
            type = types.str;
            default = "";
            description = "A script to entirely replace the unit's script.";
          };
        };
      };

      config = mkIf (fCfg.environment != null) {
        # The pull unit provisions the environment on first start and
        # refreshes it thereafter (see pullAtServiceStart). A failed
        # refresh of an existing environment exits successfully, so this
        # hard dependency only propagates initial provisioning failures.
        after = [ "flox-pull@${name}.service" ];
        requires = [ "flox-pull@${name}.service" ];
        serviceConfig = mkMerge [
          {
            Environment = common.serviceEnvironment pkgs.runtimeShell WorkingDirectory;
            ExecStart = mkForce "${execStart}";
          }
          (mkIf (fCfg.floxHubTokenFile != null) {
            LoadCredential = [ "floxhub_token:${fCfg.floxHubTokenFile}" ];
          })
        ];
      };
    };

in
{
  options = {
    # Extend every systemd service with a `flox` section for running the
    # unit from an activated Flox environment.
    systemd.services = mkOption {
      type = types.attrsOf (types.submodule floxOverridesSubmodule);
    };
  };

  config = {
    assertions =
      mapAttrsToList (name: svc: {
        assertion = svc.flox.execStart != "" || svc.flox.script != "" || svc.script != "";
        message = "systemd.services.${name}.flox.environment is set but there is no command to run. Set systemd.services.${name}.flox.execStart or systemd.services.${name}.flox.script.";
      }) floxManagedServices
      ++ mapAttrsToList (name: svc: {
        assertion = !(svc.serviceConfig.DynamicUser or false);
        message = "systemd.services.${name}: the Flox override does not support DynamicUser. Set systemd.services.${name}.serviceConfig.DynamicUser = lib.mkForce false and configure a static User and Group.";
      }) floxManagedServices;

    # Provisioning, scheduled pulls and restart-on-change are handled by
    # the flox-pull@/flox-autopull@ template units; see pull.nix.
    services.flox.pull.configs = mapAttrs (name: svc: {
      unit = "${name}.service";
      user = toString (svc.serviceConfig.User or "root");
      group = toString (svc.serviceConfig.Group or "");
      environment = svc.flox.environment;
      workingDirectory = workingDirectory name;
      inherit (svc.flox)
        extraFloxArgs
        extraFloxPullArgs
        pullAtServiceStart
        floxHubTokenFile
        ;
      autoPull = svc.flox.autoPull.enable;
      autoPullDates = svc.flox.autoPull.dates;
      autoRestart = svc.flox.autoRestart.enable;
    }) floxManagedServices;
  };
}
