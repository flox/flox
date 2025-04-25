{
  overlay = final: prev: {
    nef = final.makeScope (scope: final.callPackageWith ({ lib = final; } // final // scope)) (self: {
      dirToAttrs = (self.callPackage ./dirToAttrs.nix { }).dirToAttrs;
      extendAttrSet = (self.callPackage ./extendAttrSet.nix { }).extendAttrSet;
      mkOverlay = (self.callPackage ./mkOverlay.nix { }).mkOverlay;
      reflect = self.callPackage ./reflect.nix { };
    });
  };
}
