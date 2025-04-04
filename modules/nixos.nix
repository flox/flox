pkgsContext:
{
  config,
  options,
  pkgs,
  lib,
  system,
  ...
}:

let
  inherit (lib)
    literalExpression
    mkEnableOption
    mkIf
    mkMerge
    mkOption
    types
    ;

  programsCfg = config.programs.flox;

  # Module for installing and configuring Flox system-wide.
  floxProgramsModule =
    { ... }:
    {
      options.programs.flox = {
        enable = mkEnableOption "Flox CLI - Harness the power of Nix";
        package = mkOption {
          type = types.package;
          description = "Flox CLI package";
          default = pkgsContext.${system}.flox;
          defaultText = literalExpression "pkgs.flox";
          example = literalExpression "pkgs.flox";
        };
      };
      # Flox system-wide configuration
      config = mkMerge [
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
        (mkIf programsCfg.enable {
          environment.systemPackages = [ programsCfg.package ];
        })
      ];
    };

  /*
    The following submodules offer two ways of configuring systemd
    services to run from Flox environments:

    1. floxServiceModule: configures systemd to activate environments
       with `flox activate --start-services`, delegating all process
       management thereafter to the Flox subsystem.

       Enabled with:
         services.flox.activations = {
           foo = {
             environment = "flox/demo";
             dynamicUser = true;
           };
         };

    2. floxOverridesModule: leverages existing NixOS modules for overriding
       the `ExecStart` option to run within the activated Flox environment.

       Enabled with:
         systemd.services.echoip.flox = {
           environment = "flox/echoip";
           trustEnvironment = true;
           autoPull.enable = true;
           execStart = "echoip -l 127.0.0.1:8080 -H X-Real-IP";
         };

    While the first of these presents the easiest/most intuitive interface
    from a Flox perspective, the second makes it possible to leverage the
    rich capabilities of the NixOS module subsystem, as well as the hundreds
    of existing NixOS modules maintained by the Nix community.
  */

in
{
  imports = [
    floxProgramsModule
    ./services.nix
    ./overrides.nix
    ./autopull.nix
  ];
}
