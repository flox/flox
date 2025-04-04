{
  config,
  pkgs,
  lib,
  ...
}:

let
  inherit (lib) mkMerge;

  floxAutoPullModule = {
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
            sudo
            config.programs.flox.package
          ];
          text = ''
            confFileName="$1"
            # shellcheck disable=SC1090
            source "${autoPullConfDirectory}/$confFileName.conf"

            # $user and $environment must be set by sourcing the conf file
            # shellcheck disable=SC2154
            echo "Pulling $environment as user $user"
            xdg_tmpdir=$(mktemp -d)
            cd "$xdg_tmpdir"
            chown "$user" .
            sudo -u "$user" -EH \
              FLOX_DISABLE_METRICS=true \
              XDG_CACHE_HOME="$xdg_tmpdir"/.cache \
              XDG_DATA_HOME="$xdg_tmpdir"/.local/share \
              XDG_CONFIG_HOME="$xdg_tmpdir"/.config \
              flox pull -r "$environment"
            rm -rf "$xdg_tmpdir"
          '';
        };
      in

      mkMerge [
        {
          systemd.timers = mkMerge autoPullEnabledConfig;
          systemd.services."flox-autoPull@" = {
            serviceConfig = {
              Type = "oneshot";
              ExecStart = "${autoPullScript}/bin/flox-autoPull %i";
            };
          };
        }
      ];
  };

in
floxAutoPullModule
