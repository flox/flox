{ lib, ... }:

let
  inherit (lib) mdDoc mkOption types;

  floxServiceOpts = {
    environment = mkOption {
      type = types.nullOr types.str;
      default = null;
      example = "flox/default";
      description = mdDoc "The Flox environment to use for the service";
    };
    trustEnvironment = mkOption {
      type = types.bool;
      default = false;
      description = mdDoc "Whether to trust the environment using invocation option";
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
      example = [ "-v -v" ];
      description = mdDoc "Additional arguments to pass to `flox`";
    };
    extraFloxActivateArgs = mkOption {
      type = types.listOf types.str;
      default = [ ];
      example = [ "--mode dev" ];
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
      default = true;
      description = mdDoc "Whether to pull the Flox environment at service start";
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
        When and how often to pull updates.

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

  floxModuleOpts = {
    stateDir = mkOption {
      type = types.path;
      default = "/run/flox";
      description = ''
        Path containing all state pertaining to Flox-managed services.
      '';
    };
    workingDirectoryMode = mkOption {
      type = types.str;
      default = "0700";
      description = ''
        The mode of the service's working directory mode in numeric format.
      '';
    };
  };

in
{
  inherit floxServiceOpts floxModuleOpts;
}
