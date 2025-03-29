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
    mdDoc
    mkDefault
    mkEnableOption
    mkForce
    mkIf
    mkMerge
    mkOption
    types
    ;

  cfg = config.programs.flox;

  floxModOpts = {
    environment = mkOption {
      type = types.str;
      example = "flox/default";
      description = mdDoc "The Flox environment to use for the service";
    };
    execStart = mkOption {
      type = types.nullOr types.str;
      default = null;
      description = mdDoc "The command to override the unit's ExecStart with";
    };
    script = mkOption {
      type = types.nullOr types.str;
      default = null;
      description = mdDoc "The command to override the unit's script with";
    };
    floxHubTokenFile = mkOption {
      type = types.nullOr types.path;
      default = null;
      example = "/run/secrets/floxhub/secret.token";
      description = mdDoc "Full path to the FloxHub token file";
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

  floxMod =
    {
      options,
      config,
      name,
      ...
    }:
    let
      jobScripts = makeJobScript "${name}-start" config.flox.script;
      # Prefer config.flox.execStart over config.flox.script
      scriptAndArgs =
        if (config.flox.execStart != null && config.flox.script == null) then
          config.flox.execStart
        else
          "${jobScripts} ${config.scriptArgs}";

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
        exec -a ${cfg.package}/bin/flox ${cfg.package}/bin/flox "$@"
      '';

    in
    {
      options.flox = floxModOpts;
      config =
        mkIf
          ((config.flox.execStart != null || config.flox.script != null) && config.flox.environment != null)
          {
            serviceConfig = mkMerge [
              # Default service config
              {
                Environment = [
                  # FIXME: add flag for disabling metrics
                  "FLOX_DISABLE_METRICS=true"
                ];
                # Completely override the ExecStart config
                ExecStart = mkForce "${floxWrapper} ${escapeShellArgs config.flox.extraFloxArgs} activate -r ${config.flox.environment} ${escapeShellArgs config.flox.extraFloxActivateArgs} -- ${scriptAndArgs}";
              }

              # Workaround so the service can pull the environment from private repositories
              (mkIf (config.flox.floxHubTokenFile != null) {
                # TODO: update `flox auth login` to accept `--with-token` and
                #       read from STDIN (like `gh auth`)
                ExecStartPre = [
                  "/bin/sh -c '${floxWrapper} ${escapeShellArgs config.flox.extraFloxArgs} auth login --with-token < ${config.flox.token}'"
                ];
              })

              # Pull the environment at service start
              (mkIf (config.flox.pullAtServiceStart) {
                ExecStartPre = [
                  "${floxWrapper} ${escapeShellArgs config.flox.extraFloxArgs} pull -r ${config.flox.environment} ${escapeShellArgs config.flox.extraFloxPullArgs}"
                ];
              })
            ];
          };
    };

in
{
  # Options for installing Flox system-wide
  options = {
    programs.flox = {
      enable = mkEnableOption "Flox CLI - Harness the power of Nix";
      package = mkOption {
        type = types.package;
        description = "Flox CLI package";
        default = pkgsContext.${system}.flox;
        defaultText = literalExpression "pkgs.flox";
        example = literalExpression "pkgs.flox";
      };
    };

    # Enable floxMod submodule for overriding the execStart attribute of any
    # given systemd services to run within a Flox environment.
    systemd.services = mkOption {
      type = types.attrsOf (types.submodule floxMod);
    };
  };

  # Flox system-wide configuration
  config =
    let
      services = config.systemd.services;
      autoPullEnabledServices = lib.filterAttrs (_: value: (value.flox.autoPull.enable == true)) services;
      autoPullEnabledConfig = lib.mapAttrsToList (name: value: {
        "${name}-flox-autoPull" = {
          timerConfig = {
            RandomizedDelaySec = "15s";
            OnCalendar = value.flox.autoPull.dates;
            Unit = "flox-autoPull@${name}.service";
          };
        };
      }) autoPullEnabledServices;
      # We can't generate systemd services because that causes infinite recursion.
      # Instead pass arguments to a template service using a conf file.
      autoPullConfFiles = lib.mapAttrsToList (
        name: value:
        pkgs.writeTextFile {
          name = "${name}.conf";
          text = ''
            user="${value.serviceConfig.User}"
            environment="${value.flox.environment}"
          '';
          destination = "/${name}.conf";
        }
      ) autoPullEnabledServices;
      # put all conf files into a single directory
      autoPullConfDirectory = pkgs.buildEnv {
        name = "flox-autoPull-conf";
        paths = autoPullConfFiles;
      };
      autoPullScript = pkgs.writeShellApplication {
        name = "flox-autoPull";
        runtimeInputs = with pkgs; [
          su
          config.programs.flox.package
        ];
        text = ''
          confFileName="$1"
          # shellcheck disable=SC1090
          source "${autoPullConfDirectory}/$confFileName.conf"

          # $user and $environment must be set by sourcing the conf file
          # shellcheck disable=SC2154
          echo "Pulling $environment as user $user"
          su "$user" -s "${pkgs.bash}/bin/bash" -c "flox pull -r $environment"
        '';
      };
    in

    mkMerge [
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
      (mkIf cfg.enable {
        environment.systemPackages = [ cfg.package ];
        systemd.timers = mkMerge autoPullEnabledConfig;
        systemd.services."flox-autoPull@" = {
          serviceConfig = {
            Type = "oneshot";
            ExecStart = "${autoPullScript}/bin/flox-autoPull %i";
          };
        };
      })
    ];

}
