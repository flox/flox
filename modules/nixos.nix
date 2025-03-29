pkgsContext:
{
  config,
  lib,
  ...
}:

let
  cfg = config.programs.flox;
  floxConfigModule = import ./flox.nix pkgsContext;

in
{
  imports = [
    # Module for installing and configuring Flox system-wide.
    floxConfigModule
  ];

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [ cfg.package ];
  };
}
