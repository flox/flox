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
    escapeShellArg
    escapeShellArgs
    filterAttrs
    mapAttrs'
    mapAttrsToList
    mkIf
    mkOption
    nameValuePair
    optionalString
    types
    ;

  # Options common to both Flox module types.
  common = import ./common.nix { inherit lib; };

  pullConfigs = config.services.flox.pull.configs;

  # One pull script per managed service, keyed by systemd instance name.
  # `flox-pull@<name>.service` runs it with "start" as a dependency of the
  # main unit; `flox-autopull@<name>.service` runs it with "timer" on the
  # autoPull schedule. The script runs as root so it can create the working
  # directory and restart the main unit, but every `flox` invocation drops
  # privileges to the service user.
  pullScript =
    name: cfg:
    let
      flox = escapeShellArgs ([ "${package}/bin/flox" ] ++ cfg.extraFloxArgs);
      pullArgs = escapeShellArgs cfg.extraFloxPullArgs;
    in
    pkgs.writeShellScript "flox-pull-${name}" ''
      set -euo pipefail

      mode="''${1:?usage: flox-pull-${name} (start|timer)}"

      workdir=${escapeShellArg cfg.workingDirectory}
      user=${escapeShellArg cfg.user}
      group=${escapeShellArg cfg.group}
      if [ -z "$group" ]; then
        group="$(id -gn "$user")"
      fi

      # Ensure the working directory exists and is owned by the service user.
      mkdir -p "$workdir"
      chown "$user:$group" "$workdir"
      chmod ${workingDirectoryMode} "$workdir"
      cd "$workdir"

      # Serialize pulls against this working directory: a service-start pull
      # (flox-pull@) and a scheduled pull (flox-autopull@) are separate
      # units and may otherwise run concurrently, e.g. when a persistent
      # timer fires its catch-up run around boot.
      exec 9>.flox-pull.lock
      flock 9

      ${optionalString (cfg.floxHubTokenFile != null) ''
        # Export the FloxHub token for the flox invocations below. The token
        # must only ever travel through the environment: putting it on a
        # command line would expose it in /proc.
        FLOX_FLOXHUB_TOKEN="$(cat ${escapeShellArg cfg.floxHubTokenFile})"
        export FLOX_FLOXHUB_TOKEN
      ''}

      # Run a command as the service user, preserving the environment
      # (including an exported FLOX_FLOXHUB_TOKEN).
      as_user() {
        setpriv --reuid "$user" --regid "$group" --init-groups \
          env ${
            escapeShellArgs (
              common.serviceEnvironment pkgs.runtimeShell cfg.workingDirectory cfg.user metrics.enable
            )
          } "$@"
      }

      # Fingerprint of the local generation history, used to detect whether
      # a pull fetched a new generation.
      generation_state() {
        as_user ${flox} generations list --json 2>/dev/null | sha256sum
      }

      ${optionalString (cfg.generation != null) ''
        # Pinned to one generation: provision a detached copy of exactly
        # that generation, then leave it alone until the pin changes.
        pin=${toString cfg.generation}
        marker="$workdir/.flox-pinned-generation"

        if [ "$mode" = "timer" ]; then
          # Scheduled pulls never move a pinned service.
          exit 0
        fi
        if [ -e "$workdir/.flox" ] && [ "$(cat "$marker" 2>/dev/null)" = "$pin" ]; then
          exit 0
        fi

        # Keep the current environment until the pinned one is in place,
        # so a failed pull never leaves the service without one.
        rm -rf "$workdir/.flox.prev"
        if [ -e "$workdir/.flox" ]; then
          mv "$workdir/.flox" "$workdir/.flox.prev"
        fi
        if as_user ${flox} pull ${pullArgs} --copy --generation "$pin" ${escapeShellArg cfg.environment}; then
          rm -rf "$workdir/.flox.prev"
          echo "$pin" > "$marker"
          exit 0
        fi
        rm -rf "$workdir/.flox"
        if [ -e "$workdir/.flox.prev" ]; then
          mv "$workdir/.flox.prev" "$workdir/.flox"
          echo "WARNING: failed to pull generation $pin of ${cfg.environment}; starting with the existing environment." >&2
          exit 0
        fi
        echo "ERROR: failed to pull generation $pin of ${cfg.environment}." >&2
        exit 1
      ''}

      ${optionalString (cfg.generation == null) ''
        # Un-pinned after having been pinned: the existing environment is a
        # detached copy with no link to FloxHub, so `flox pull --force` could
        # never update it. Re-provision a linked environment instead.
        if [ -e "$workdir/.flox-pinned-generation" ] && [ -e "$workdir/.flox" ]; then
          rm -rf "$workdir/.flox.prev"
          mv "$workdir/.flox" "$workdir/.flox.prev"
          if as_user ${flox} pull ${pullArgs} ${escapeShellArg cfg.environment}; then
            rm -rf "$workdir/.flox.prev" "$workdir/.flox-pinned-generation"
            exit 0
          fi
          rm -rf "$workdir/.flox"
          mv "$workdir/.flox.prev" "$workdir/.flox"
          echo "WARNING: failed to re-link ${cfg.environment} to its live generation; still running the previously pinned copy." >&2
          exit 0
        fi
      ''}

      if [ ! -e "$workdir/.flox" ]; then
        # First start: provision the environment. Failure is fatal - there
        # is nothing to run without an environment.
        as_user ${flox} pull ${pullArgs} ${escapeShellArg cfg.environment}
        exit 0
      fi

      ${optionalString (!cfg.pullAtServiceStart) ''
        if [ "$mode" = "start" ]; then
          # The environment is provisioned and pullAtServiceStart is
          # disabled: nothing to do at service start.
          exit 0
        fi
      ''}

      before="$(generation_state)" || before=""
      if ! as_user ${flox} pull --force ${pullArgs}; then
        if [ "$mode" = "timer" ]; then
          echo "ERROR: failed to pull updates for ${cfg.environment}." >&2
          exit 1
        fi
        # The environment is already present; a failed refresh must not
        # prevent the service from (re)starting.
        echo "WARNING: failed to pull updates for ${cfg.environment}; starting with the existing environment." >&2
        exit 0
      fi
      after="$(generation_state)" || after=""

      ${optionalString cfg.autoRestart ''
        if [ "$mode" = "timer" ] && [ "$before" != "$after" ]; then
          echo "Flox environment ${cfg.environment} changed; restarting ${cfg.unit}" >&2
          systemctl try-restart ${escapeShellArg cfg.unit}
        fi
      ''}
    '';

  scriptsDir = pkgs.linkFarm "flox-pull-scripts" (
    mapAttrsToList (name: cfg: {
      inherit name;
      path = pullScript name cfg;
    }) pullConfigs
  );

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
      ExecStart = "${scriptsDir}/%i ${mode}";
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
        Per-service pull configuration, contributed by the Services and
        Overrides modules and consumed by the `flox-pull@` and
        `flox-autopull@` template units.
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
            workingDirectory = mkOption { type = types.str; };
            extraFloxArgs = mkOption {
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
            generation = mkOption {
              type = types.nullOr types.ints.positive;
              default = null;
            };
          };
        }
      );
    };
  };

  config = mkIf enable {
    # The templates are inert without instances, so instances are not part
    # of the condition here. It must stay a plain user-set flag: gating on
    # `pullConfigs != { }` would make the merge of `systemd.services`
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
    ) (filterAttrs (_: cfg: cfg.autoPull) pullConfigs);

    # Parent directory for the per-service working directories. The pull
    # script creates and owns each service's directory beneath it.
    systemd.tmpfiles.rules = [
      "d ${stateDir} 0755 root root - -"
    ];
  };
}
