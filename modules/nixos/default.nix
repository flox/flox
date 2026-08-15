pkgsContext:
{
  config,
  lib,
  ...
}:

let
  cfg = config.programs.flox;

in
{
  imports = [
    # Module for installing and configuring Flox system-wide.
    (import ../flox.nix pkgsContext)

    /*
      The following submodules offer two ways of configuring systemd
      services to run from Flox environments:

      1. services.nix: configures systemd to activate environments
         with `flox activate --start-services`, delegating all process
         management thereafter to the Flox subsystem.

         Enabled with:
           services.flox = {
             enable = true;
             activations = {
               myechoip = {
                 environment = "flox/echoip";
                 trustEnvironment = true;
                 autoPull.enable = true;
                 floxHubTokenFile = "/run/keys/echoip.token";
               };
             };
           };

      2. overrides.nix: leverages existing NixOS modules for overriding
         the `ExecStart` option to run within the activated Flox
         environment.

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

      Both are backed by pull.nix, which provisions each service's
      environment, refreshes it on service start and on a schedule, and
      optionally restarts the service when a pull fetches a new generation.
    */
    ./pull.nix
    ./services.nix
    ./overrides.nix
  ];

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [ cfg.package ];
  };
}
