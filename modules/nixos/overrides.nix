{
  config,
  options,
  pkgs,
  lib,
  name,
  utils,
  ...
}:

let
  inherit (config.programs.flox) package;
  inherit (config.services.flox) stateDir workingDirectoryMode;
  inherit (utils.systemdUtils.lib) makeJobScript;
  inherit (lib)
    escapeShellArgs
    mdDoc
    mkForce
    mkIf
    mkMerge
    mkOption
    types
    ;

  # Options common to both Flox module types.
  common = import ./common.nix { inherit lib; };

  # Function to calculate the working directory for a service.
  workingDirectory = name: "${stateDir}/${name}";

  floxOverridesSubmodule =
    {
      options,
      config,
      name,
      ...
    }:
    let
      WorkingDirectory = workingDirectory name;

      jobScripts = makeJobScript {
        name = "${name}-start";
        text =
          if (config.flox.script != "") then
            config.flox.script
          else if (config.script != "") then
            config.script
          else
            "";
        inherit (config) enableStrictShellChecks;
      };
      # Prefer config.flox.execStart over config{,.flox}.script.
      scriptAndArgs =
        if (config.flox.execStart != "") then
          config.flox.execStart
        else if (jobScripts != "") then
          "${jobScripts} ${config.scriptArgs}"
        else
          null;

      # Variables to customize flox invocations.
      floxWithArgs = [ "${package}/bin/flox" ] ++ config.flox.extraFloxArgs;

      floxAuthLoginWithArgs =
        escapeShellArgs (
          floxWithArgs
          ++ [
            "config"
            "--set"
            "floxhub_token"
          ]
        )
        + " \"$(cat ${config.flox.floxHubTokenFile})\"";

      floxPullWithArgs = escapeShellArgs (
        floxWithArgs
        ++ [
          "pull"
          "--force"
          config.flox.environment
        ]
        ++ config.flox.extraFloxPullArgs
      );

      # We need a wrapper to optionally authenticate and pull updates prior to
      # invoking `flox activate`. It would be better to do this like we do in
      # services.nix by configuring separate service units for these functions,
      # but I haven't found a way to declare new services from within a submodule.
      ExecStartPre = pkgs.writeScript "${name}-ExecStartPre" (
        ''
          #! ${pkgs.runtimeShell} -eu
          if [ -d ${WorkingDirectory} ]; then
            # Assert all files in working directory are writable by the user.
            # We won't be able to fix them, but at least we'll know.
            myid=$(id -u)
            find -H ${WorkingDirectory} '!' -user $myid -print0 | \
              xargs --null --no-run-if-empty echo -e \
                "WARNING: files with dubious ownership found in ${WorkingDirectory}\n" \
                "--> FIX WITH: chown $myid"
          else
            ( set -x && mkdir -p ${WorkingDirectory} )
          fi
          # Be verbose about actions from this point forward.
          set -x
          chmod ${workingDirectoryMode} ${WorkingDirectory}
          cd ${WorkingDirectory}
        ''
        + lib.optionalString (config.flox.floxHubTokenFile != null) ''
          ${pkgs.runtimeShell} -c '${floxAuthLoginWithArgs}'
        ''
        + lib.optionalString (config.flox.pullAtServiceStart) floxPullWithArgs
      );

      floxActivateWithArgs = escapeShellArgs (
        floxWithArgs
        ++ [
          "activate"
          "--dir"
          WorkingDirectory
        ]
        ++ lib.optionals config.flox.trustEnvironment [ "--trust" ]
        ++ config.flox.extraFloxActivateArgs
      );

    in
    {
      options = {
        flox =
          common.floxServiceOpts
          // common.floxModuleOpts
          // {
            execStart = mkOption {
              type = types.str;
              default = "";
              description = mdDoc "The command to override the unit's ExecStart with";
            };
            script = mkOption {
              type = types.str;
              default = "";
              description = mdDoc "A script to entirely replace the unit's script";
            };
          };
      };
      config = mkIf (config.flox.environment != null) {
        serviceConfig = mkMerge [
          # Default service config
          {
            Environment = [
              "FLOX_DISABLE_METRICS=true"
              "HOME=${WorkingDirectory}"
              "SHELL=${pkgs.runtimeShell}"
            ];
            inherit ExecStartPre;
          }
          (mkIf (scriptAndArgs != null) {
            # Completely override the ExecStart config
            ExecStart = mkForce "${floxActivateWithArgs} -- ${scriptAndArgs}";
          })
        ];
      };
    };

  floxOverridesModule = {
    options = {
      # Enable floxOverridesSubmodule for overriding the execStart attribute
      # of any given systemd service to run within a Flox environment.
      systemd.services = mkOption {
        type = types.attrsOf (types.submodule floxOverridesSubmodule);
      };
    };
    config = {
      # Create a state directory for storing "project" directories for the services
      # to be activated. Create this using mode 1777 (like /tmp) so that services
      # not running as root can create their own directories within it.
      systemd.tmpfiles.rules = [
        "d ${stateDir} 1777 root root - -"
      ];
    };
  };

in
floxOverridesModule
