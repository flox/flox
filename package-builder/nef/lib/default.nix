{
  overlay = final: prev: {
    nef = final.makeScope (scope: final.callPackageWith ({ lib = final; } // final // scope)) (
      self:
      let
        extendAttrSetModule = self.callPackage ./extendAttrSet.nix { };
        mkOverlayModule = self.callPackage ./mkOverlay.nix { };
      in
      {
        dirToAttrs = (self.callPackage ./dirToAttrs.nix { }).dirToAttrs;
        inherit (extendAttrSetModule) extendAttrSet applyOverlay;
        inherit (mkOverlayModule) mkOverlay callPackageIn;
        reflect = self.callPackage ./reflect.nix { };
        deepOverrides = self.callPackage ./deepOverrides.nix { };
        instantiate = self.callPackage ./instantiate.nix { };
      }
    );
  };
}
