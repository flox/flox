# options and implementation for all POSIX shells
{
  config,
  lib,
  context,
  system,
  ...
}: let
  floxpkgs = context.inputs.flox-floxpkgs;
  pkgs = context.nixpkgs;
in
  with lib; {
    # common options for POSIX compatible shells
    options = {
      shell = {
        aliases = mkOption {
          default = {};
          example = {
            ll = "ls -l";
          };
          description = lib.mdDoc ''
            An attribute set that maps aliases (the top level attribute names in
            this option) to command strings.

            Aliases can also be mapped directly to packages, and aliases mapped
            to `null` are ignored.
          '';
          type = with types; attrsOf (nullOr (either str path));
        };
        hook = mkOption {
          default = "";
          description = lib.mdDoc ''
            Shell script code called during environment activation.
            This code is assumed to be shell-independent, which means you should
            stick to pure sh without sh word split.
          '';
          type = types.lines;
          example = ''
            echo "Supercharged by flox!" 1>&2
          '';
        };
      };
    };

    config = let
      stringAliases = concatStringsSep "\n" (
        mapAttrsFlatten (k: v: "alias ${k}=${escapeShellArg v}")
        (filterAttrs (k: v: v != null) config.shell.aliases)
      );

      exportedEnvVars = let
        exportVariables =
          if builtins.isList config.environmentVariables
          then let
            # double quote and replace " with \"
            escapeShellArgToEval = arg: "\"${lib.replaceStrings [''"''] [''\"''] arg}\"";
          in
            builtins.concatLists (builtins.map
              # don't escape variables defined in a list
              (envAttrSet: mapAttrsToList (n: v: ''export ${n}=${escapeShellArgToEval v}'') envAttrSet)
              config.environmentVariables)
          else (mapAttrsToList (n: v: ''export ${n}=${escapeShellArg v}'') config.environmentVariables);
      in
        concatStringsSep "\n" exportVariables;
      activateScript = pkgs.writeTextFile {
        name = "activate";
        executable = true;
        destination = "/activate";
        text = ''
          ${exportedEnvVars}

          ${stringAliases}

          ${config.shell.hook}
        '';
      };
    in {
      passthru.posix = floxpkgs.lib.mkEnv {
        inherit pkgs;
        packages = config.packagesList ++ [config.newCatalogPath activateScript];
        manifestPath = config.manifestPath;
        meta.buildLayeredImageArgs = config.passthru.buildLayeredImageArgs;
      };
      toplevel = config.passthru.posix // {passthru = config.passthru;} // config.passthru;
    };
  }
