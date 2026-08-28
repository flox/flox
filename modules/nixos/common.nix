{ lib, ... }:

let
  inherit (lib) boolToString mkOption types;

  # Options shared by both the Services method (`services.flox.activations`)
  # and the Overrides method (`systemd.services.<name>.flox`). The
  # `environment` option is declared separately by each method: it is
  # required for activations, while for overrides a null value means the
  # unit is left untouched.
  floxServiceOpts = {
    trustEnvironment = mkOption {
      type = types.bool;
      default = false;
      description = "Whether to pass `--trust` when activating the environment.";
    };
    floxHubTokenFile = mkOption {
      type = types.nullOr types.path;
      default = null;
      example = "/run/secrets/floxhub/secret.token";
      description = ''
        Full path to a file containing a FloxHub token.
        The token is exported to `flox` via the `FLOX_FLOXHUB_TOKEN`
        environment variable and never appears on a command line or in a
        configuration file.
        The file must be readable by root; the service itself receives the
        token through a systemd credential.
      '';
    };
    extraFloxArgs = mkOption {
      type = types.listOf types.str;
      default = [ ];
      example = [
        "-v"
        "-v"
      ];
      description = "Additional arguments to pass to every `flox` invocation.";
    };
    extraFloxActivateArgs = mkOption {
      type = types.listOf types.str;
      default = [ ];
      example = [
        "--mode"
        "dev"
      ];
      description = "Additional arguments to pass to `flox activate`.";
    };
    extraFloxPullArgs = mkOption {
      type = types.listOf types.str;
      default = [ ];
      example = [ "-v" ];
      description = "Additional arguments to pass to `flox pull`.";
    };
    pullAtServiceStart = mkOption {
      type = types.bool;
      default = true;
      description = ''
        Whether to refresh the Flox environment every time the service
        starts.
        The initial provisioning pull always happens regardless of this
        option.
        A failed refresh of an already-provisioned environment does not
        prevent the service from starting.
      '';
    };
    autoPull.enable = mkOption {
      type = types.bool;
      default = false;
      description = "Whether to pull the Flox environment on a schedule.";
    };
    autoPull.dates = mkOption {
      type = types.str;
      default = "00:00";
      example = "daily";
      description = ''
        When and how often to pull updates.

        The format is described in
        {manpage}`systemd.time(7)`.
      '';
    };
    autoRestart.enable = mkOption {
      type = types.bool;
      default = false;
      description = ''
        Whether to restart the service when a scheduled pull
        (see `autoPull`) fetches a new generation of the environment.
        Without this option a pulled update only takes effect the next
        time the service is restarted.
      '';
    };
  };

  floxModuleOpts = {
    stateDir = mkOption {
      type = types.path;
      default = "/var/lib/flox";
      description = ''
        Path containing all state pertaining to Flox-managed services.
        Each service gets a working directory beneath it, holding the
        pulled environment.
      '';
    };
    workingDirectoryMode = mkOption {
      type = types.str;
      default = "0700";
      description = ''
        The mode of each service's working directory in numeric format.
      '';
    };
    metrics.enable = mkOption {
      type = types.bool;
      default = true;
      description = ''
        Whether to let `flox` submit metrics for Flox-managed services.
        Each service runs with its own Flox configuration directory, so
        this is the only place the setting can be made for them.
      '';
    };
  };

  # Environment for every process that invokes flox on behalf of a service.
  # USER must match the passwd name of the invoking uid: when it does not
  # (e.g. under setpriv or a oneshot unit), flox resets HOME from the
  # passwd database, discarding the working-directory HOME set here. The
  # XDG variables are additionally pinned beneath the working directory so
  # flox state stays with the service even if HOME is reset anyway.
  # The pinned XDG_CONFIG_HOME gives each service an empty Flox
  # configuration directory, so FLOX_DISABLE_METRICS is the only way
  # `services.flox.metrics.enable` can reach it.
  serviceEnvironment = shell: workingDirectory: user: metrics: [
    "FLOX_DISABLE_METRICS=${boolToString (!metrics)}"
    "HOME=${workingDirectory}"
    "LOGNAME=${user}"
    "SHELL=${shell}"
    "USER=${user}"
    "XDG_CACHE_HOME=${workingDirectory}/.cache"
    "XDG_CONFIG_HOME=${workingDirectory}/.config"
    "XDG_DATA_HOME=${workingDirectory}/.local/share"
    "XDG_STATE_HOME=${workingDirectory}/.local/state"
  ];

in
{
  inherit floxServiceOpts floxModuleOpts serviceEnvironment;
}
