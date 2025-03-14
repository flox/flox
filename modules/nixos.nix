floxOverlay:
{
  config,
  pkgs,
  lib,
  ...
}:

let
  cfg = config.programs.flox;
  # "Extract" _only_ flox package from floxOverlay.
  flox = (import pkgs.path { overlays = floxOverlay; }).flox;
in
{

  options = {
    programs.flox = {
      enable = lib.mkEnableOption "Flox CLI - Harness the power of Nix";
      package = lib.mkPackageOption pkgs "Flox CLI package" {
        default = [ "flox" ];
      };
    };
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [ cfg.package ];
    nixpkgs.overlays = [ (f: p: { inherit flox; }) ];
    nix.settings = {
      trusted-public-keys = lib.mkAfter [
        "flox-cache-public-1:7F4OyH7ZCnFhcze3fJdfyXYLQw/aV7GEed86nQ7IsOs="
      ];
      substituters = lib.mkAfter [
        "https://cache.flox.dev"
      ];
    };

  };
}
