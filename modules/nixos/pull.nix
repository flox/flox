{
  config,
  pkgs,
  lib,
  ...
}:

let
  inherit (config.programs.flox) package;
  inherit (config.services.flox)
    enable
    metrics
    stateDir
    workingDirectoryMode
    ;
  inherit (lib)
    boolToString
    filterAttrs
    mapAttrs'
    mapAttrsToList
    mkIf
    mkOption
    nameValuePair
    types
    ;

  serviceConfigs = config.services.flox.pull.configs;

  conf = import ./conf.nix { inherit lib; };

  # The entry points are shared verbatim with the distro-agnostic integration
  # in ../systemd, so NixOS and a Debian or RHEL host run the same code. This
  # module's remaining job is to render each service's configuration file and
  # point the units at these scripts; see ../systemd/README.md for what the
  # scripts expect.
  scripts = pkgs.runCommand "flox-systemd-scripts" { } ''
    mkdir -p "$out/libexec/flox"
    cp ${../systemd/libexec}/* "$out/libexec/flox/"
    chmod +x "$out/libexec/flox/flox-pull" \
             "$out/libexec/flox/flox-activate" \
             "$out/libexec/flox/flox-exec-start"
    patchShebangs "$out/libexec/flox"
  '';

  libexec = "${scripts}/libexec/flox";

  # The rendered per-service configuration. This lives in the store rather
  # than /etc: defining `environment.etc` in terms of `pull.configs` makes it
  # depend on `config.systemd.services`, and other NixOS modules' services
  # read `environment.etc` back, which is an evaluation cycle.
  confDir = pkgs.linkFarm "flox-service-configs" (
    mapAttrsToList (name: cfg: {
      name = "${name}.conf";
      path = pkgs.writeText "flox-service-${name}.conf" (conf.mkConf cfg);
    }) serviceConfigs
  );

  # Everything the scripts need in order to find flox, their configuration and
  # the state directory. The per-service settings travel in the conf files
  # rather than here, so these are identical for every unit.
  staticScriptEnvironment = [
    "FLOX_BIN=${package}/bin/flox"
    "FLOX_DISABLE_METRICS=${boolToString (!metrics.enable)}"
    "FLOX_LIBEXEC=${libexec}"
    "FLOX_STATE_DIR=${stateDir}"
    "FLOX_WORKDIR_MODE=${workingDirectoryMode}"
    "SHELL=${pkgs.runtimeShell}"
  ];

  scriptEnvironment = staticScriptEnvironment ++ [ "FLOX_CONF_DIR=${confDir}" ];

  templateUnit = mode: {
    description = "Flox environment pull for %i (${mode})";
    # Pulling needs the network; at boot the main unit may otherwise pull
    # this in before the network is up.
    wants = [ "network-online.target" ];
    after = [ "network-online.target" ];
    path = [
      pkgs.coreutils
      pkgs.util-linux
    ];
    serviceConfig = {
      Type = "oneshot";
      Environment = scriptEnvironment;
      ExecStart = "${libexec}/flox-pull %i ${mode}";
    };
  };

in
{
  options = {
    services.flox.pull.configs = mkOption {
      internal = true;
      visible = false;
      default = { };
      description = ''
        Per-service configuration, contributed by the Services and Overrides
        modules, rendered to `/etc/flox/services/<name>.conf` and consumed by
        the shared entry points and the `flox-pull@` and `flox-autopull@`
        template units.
      '';
      type = types.attrsOf (
        types.submodule {
          options = {
            unit = mkOption { type = types.str; };
            user = mkOption { type = types.str; };
            # An empty group means the user's primary group.
            group = mkOption {
              type = types.str;
              default = "";
            };
            environment = mkOption { type = types.str; };
            trustEnvironment = mkOption {
              type = types.bool;
              default = false;
            };
            # Method 2 only: the command to run inside the activation. Empty
            # for an activation that starts the environment's own services.
            execStart = mkOption {
              type = types.str;
              default = "";
            };
            extraFloxArgs = mkOption {
              type = types.listOf types.str;
              default = [ ];
            };
            extraFloxActivateArgs = mkOption {
              type = types.listOf types.str;
              default = [ ];
            };
            extraFloxPullArgs = mkOption {
              type = types.listOf types.str;
              default = [ ];
            };
            pullAtServiceStart = mkOption {
              type = types.bool;
              default = true;
            };
            autoPull = mkOption {
              type = types.bool;
              default = false;
            };
            autoPullDates = mkOption {
              type = types.str;
              default = "00:00";
            };
            autoRestart = mkOption {
              type = types.bool;
              default = false;
            };
            floxHubTokenFile = mkOption {
              type = types.nullOr types.path;
              default = null;
            };
          };
        }
      );
    };

    services.flox.libexec = mkOption {
      internal = true;
      visible = false;
      readOnly = true;
      type = types.str;
      default = libexec;
      description = "Directory holding the shared Flox systemd entry points.";
    };

    services.flox.staticScriptEnvironment = mkOption {
      internal = true;
      visible = false;
      readOnly = true;
      type = types.listOf types.str;
      default = staticScriptEnvironment;
      description = ''
        The part of the entry-point environment that does not depend on
        `pull.configs`. The Overrides method must use this: it derives its
        configs from `systemd.services`, so referencing anything built from
        `pull.configs` inside a unit would be self-referential.
      '';
    };

    services.flox.scriptEnvironment = mkOption {
      internal = true;
      visible = false;
      readOnly = true;
      type = types.listOf types.str;
      default = scriptEnvironment;
      description = "Environment the shared entry points need in every unit.";
    };
  };

  config = mkIf enable {
    # The templates are inert without instances, so instances are not part
    # of the condition here. It must stay a plain user-set flag: gating on
    # `serviceConfigs != { }` would make the merge of `systemd.services`
    # depend on its own values and diverge.
    systemd.services."flox-pull@" = templateUnit "start";
    systemd.services."flox-autopull@" = templateUnit "timer";

    systemd.timers = mapAttrs' (
      name: cfg:
      nameValuePair "flox-autopull@${name}" {
        wantedBy = [ "timers.target" ];
        timerConfig = {
          OnCalendar = cfg.autoPullDates;
          RandomizedDelaySec = "15s";
          Persistent = true;
        };
      }
    ) (filterAttrs (_: cfg: cfg.autoPull) serviceConfigs);

    # Parent directory for the per-service working directories. The pull
    # script creates and owns each service's directory beneath it.
    systemd.tmpfiles.rules = [
      "d ${stateDir} 0755 root root - -"
    ];
  };
}
