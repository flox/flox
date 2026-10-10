{
  config,
  pkgs,
  lib,
  utils,
  ...
}:

let
  floxCfg = config.services.flox;
  inherit (config.services.flox) enable stateDir;
  inherit (utils.systemdUtils.lib) makeJobScript;
  inherit (lib)
    filterAttrs
    mapAttrs
    mapAttrsToList
    mkForce
    mkIf
    mkMerge
    mkOption
    types
    ;

  # Options common to both Flox module types.
  common = import ./common.nix { inherit lib; };
  conf = import ./conf.nix { inherit lib; };

  # Function to calculate the working directory for a service.
  workingDirectory = name: "${stateDir}/${name}";

  # Units that have been handed over to Flox. Only ever used to define
  # options outside of `systemd.services` (pull configs, assertions):
  # feeding it back into `systemd.services` would make the merge depend on
  # its own result.
  floxManagedServices = filterAttrs (_: svc: svc.flox.environment != null) config.systemd.services;

  # The command the unit should run inside the activation. Prefer
  # flox.execStart, then flox.script, then the unit's own script. The null
  # case is rejected by an assertion below; the "false" placeholder only
  # guards stray evaluations that bypass the assertion machinery.
  #
  # This is computed from the evaluated service rather than inside the
  # submodule because it travels to the shared entry point through the
  # service's conf file, which is rendered outside `systemd.services`.
  mainCommand =
    name: svc:
    let
      scriptText = if svc.flox.script != "" then svc.flox.script else svc.script;
      jobScript = makeJobScript {
        name = "${name}-flox-start";
        text = scriptText;
        inherit (svc) enableStrictShellChecks;
      };
    in
    if svc.flox.execStart != "" then
      svc.flox.execStart
    else if scriptText != "" then
      "${jobScript} ${svc.scriptArgs}"
    else
      "false";

  floxOverridesSubmodule =
    {
      config,
      name,
      ...
    }:
    let
      fCfg = config.flox;
      WorkingDirectory = workingDirectory name;

      # This unit's configuration, named directly rather than looked up in the
      # shared directory: that directory is built from `pull.configs`, which is
      # derived from `systemd.services`, so reaching it from inside a unit
      # would be self-referential.
      confFile = pkgs.writeText "flox-service-${name}.conf" (
        conf.mkConf {
          inherit (fCfg)
            environment
            trustEnvironment
            extraFloxArgs
            extraFloxActivateArgs
            extraFloxPullArgs
            pullAtServiceStart
            floxHubTokenFile
            ;
          unit = "${name}.service";
          user = toString (config.serviceConfig.User or "root");
          group = toString (config.serviceConfig.Group or "");
          execStart = mainCommand name config;
          autoRestart = fCfg.autoRestart.enable;
        }
      );
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

      config = mkIf (enable && fCfg.environment != null) {
        # The pull unit provisions the environment on first start and
        # refreshes it thereafter (see pullAtServiceStart). A failed
        # refresh of an existing environment exits successfully, so this
        # hard dependency only propagates initial provisioning failures.
        after = [ "flox-pull@${name}.service" ];
        requires = [ "flox-pull@${name}.service" ];
        serviceConfig = mkMerge [
          {
            # The shared entry point applies HOME, USER and the XDG variables
            # itself, so they are not repeated here; see ../systemd/libexec.
            Environment = floxCfg.staticScriptEnvironment ++ [ "FLOX_CONF_FILE=${confFile}" ];
            ExecStart = mkForce "${floxCfg.libexec}/flox-exec-start ${name}";
            # Upstream modules commonly harden their units with
            # ProtectSystem=strict; the activation must still be able to
            # write the working directory and reach the nix daemon.
            ReadWritePaths = [
              WorkingDirectory
              "/nix/var/nix/daemon-socket"
            ];
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
      mapAttrsToList (name: _: {
        assertion = enable;
        message = "systemd.services.${name}.flox.environment is set but services.flox.enable is false. Set services.flox.enable = true to run units from Flox environments.";
      }) floxManagedServices
      ++ mapAttrsToList (name: svc: {
        assertion = svc.flox.execStart != "" || svc.flox.script != "" || svc.script != "";
        message = "systemd.services.${name}.flox.environment is set but there is no command to run. Set systemd.services.${name}.flox.execStart or systemd.services.${name}.flox.script.";
      }) floxManagedServices
      ++ mapAttrsToList (name: svc: {
        assertion = !(svc.serviceConfig.DynamicUser or false);
        message = "systemd.services.${name}: the Flox override does not support DynamicUser. Set systemd.services.${name}.serviceConfig.DynamicUser = lib.mkForce false and configure a static User and Group.";
      }) floxManagedServices;

    # Provisioning, scheduled pulls and restart-on-change are handled by
    # the flox-pull@/flox-autopull@ template units; see pull.nix.
    services.flox.pull.configs = mkIf enable (
      mapAttrs (name: svc: {
        unit = "${name}.service";
        user = toString (svc.serviceConfig.User or "root");
        group = toString (svc.serviceConfig.Group or "");
        environment = svc.flox.environment;
        inherit (svc.flox)
          extraFloxArgs
          extraFloxPullArgs
          pullAtServiceStart
          floxHubTokenFile
          ;
        execStart = mainCommand name svc;
        trustEnvironment = svc.flox.trustEnvironment;
        extraFloxActivateArgs = svc.flox.extraFloxActivateArgs;
        autoPull = svc.flox.autoPull.enable;
        autoPullDates = svc.flox.autoPull.dates;
        autoRestart = svc.flox.autoRestart.enable;
      }) floxManagedServices
    );
  };
}
