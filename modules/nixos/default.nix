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

in
{
  # Module for installing and configuring Flox system-wide.
  imports = [
    /*
      The following submodules offer two ways of configuring systemd
      services to run from Flox environments:

      1. floxServiceModule: configures systemd to activate environments
         with `flox activate --start-services`, delegating all process
         management thereafter to the Flox subsystem.

         Enabled with:
           services.flox = {
             enable = true;
             activations = {
               myechoip = {
                 environment = "flox/echoip";
                 trustEnvironment = true;
                 autoPull = true;
                 floxHubTokenFile = "/run/keys/echoip.token";
                 dynamicUser = true;
               };
             };
           };

      2. floxOverridesModule: leverages existing NixOS modules for overriding
         the `ExecStart` option to run within the activated Flox environment.

         Enabled with:
           systemd.services.echoip.flox = {
             environment = "flox/echoip";
             trustEnvironment = true;
             autoPull = true;
             execStart = "echoip -l 127.0.0.1:8080 -H X-Real-IP";
           };

      While the first of these presents the easiest/most intuitive interface
      from a Flox perspective, the second makes it possible to leverage the
      rich capabilities of the NixOS module subsystem, as well as the hundreds
      of existing NixOS modules maintained by the Nix community.
    */
    ./services.nix
    ./overrides.nix

    /*
      floxAutoPullModule: creates a service configured to automatically
      pull updates to the flox environments used for systemd services,
      optionally restarting those services when this occurs.
    */
    ./autopull.nix
  ];

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
}
