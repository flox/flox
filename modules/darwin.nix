pkgsContext:
{
  config,
  lib,
  system,
  ...
}:

let
  cfg = config.programs.flox;
in
{

  options = {
    programs.flox = {
      enable = lib.mkEnableOption "Flox CLI - Harness the power of Nix";
      package = lib.mkOption {
        type = lib.types.package;
        description = "Flox CLI package";
        default = pkgsContext.${system}.flox;
        defaultText = lib.literalExpression "pkgs.flox";
        example = lib.literalExpression "pkgs.flox";
      };
    };
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [ cfg.package ];
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
