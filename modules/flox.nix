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
        default = pkgsContext.${pkgs.stdenv.hostPlatform.system}.flox;
        defaultText = lib.literalExpression "pkgs.flox";
        example = lib.literalExpression "pkgs.flox";
      };
    };
  };

  config = {
    nix.settings = {
      trusted-public-keys = lib.mkAfter [
        "flox-cache-public-1:7F4OyH7ZCnFhcze3fJdfyXYLQw/aV7GEed86nQ7IsOs="
        # Packages published to FloxHub-hosted catalogs are signed with floxhub-1.
        # The Flox installer ships this key; without it, a system that gets Flox
        # from this module cannot install a published package ("is not signed by
        # a trusted key"). https://flox.dev/docs/customer/signing-keys/
        "floxhub-1:0QOAlcobcEvq1mqEf4qAYCaWnTTOXpyoRv/PmqfSixM="
      ];
      substituters = lib.mkAfter [
        "https://cache.flox.dev"
      ];
    };
  };

}
