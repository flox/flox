# at this point just a thin wrapper around streamLayeredImage, but still worth
# having to keep as much functionality as possible in lib rather than the module
# system
{
  inputs,
  lib,
}: drv: let
  pkgs = inputs.nixpkgs.legacyPackages.${drv.system}.pkgs;
  buildLayeredImageArgs =
    lib.recursiveUpdate
    {
      name = drv.name;
      # symlinkJoin fails when drv contains a symlinked bin directory, so wrap in an additional buildEnv
      contents = pkgs.buildEnv {
        name = "contents";
        paths = [drv pkgs.bashInteractive pkgs.coreutils];
      };
      config = {
        # By default, match the existing semantics of Nixpkgs
        Entrypoint = [
          "${drv.outPath}/bin/${drv.meta.mainProgram
            or (with builtins; parseDrvName (unsafeDiscardStringContext drv.name)).name}"
        ];
      };
    }
    (drv.meta.buildLayeredImageArgs or {});
in
  inputs.nixpkgs.legacyPackages.${drv.system}.pkgs.dockerTools.streamLayeredImage buildLayeredImageArgs
