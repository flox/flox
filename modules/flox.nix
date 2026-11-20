pkgsContext:
{
  config,
  pkgs,
  lib,
  ...
}:

let
  cfg = config.programs.flox;
  system = pkgs.stdenv.hostPlatform.system;
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
        # Without the fallback, a configuration on a system the flake dropped
        # (x86_64-darwin) fails with "attribute missing" and no pointer to why.
        default =
          pkgsContext.${system}.flox or (throw ''
            Flox does not ship the CLI for ${system}; support for x86_64-darwin (Intel Macs) has ended.
            The Flox flake provides it for: ${lib.concatStringsSep ", " (builtins.attrNames pkgsContext)}.
            To keep this configuration evaluating, set `programs.flox.package` to a Flox package
            you provide, or set `programs.flox.enable = false`.
          '');
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
