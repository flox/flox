pkgsContext:
{
  config,
  pkgs,
  lib,
  ...
}:

let
  cfg = config.programs.flox;
in
{

  options = {
    programs.flox = {
      enable = lib.mkOption {
        default = true;
        example = true;
        description = "Whether to enable Flox CLI";
        type = lib.types.bool;
      };
      package = lib.mkOption {
        type = lib.types.package;
        description = "Flox CLI package";
        default = pkgsContext.${pkgs.system}.flox;
        defaultText = lib.literalExpression "pkgs.flox";
        example = lib.literalExpression "pkgs.flox";
      };
    };
  };

  config = {
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
