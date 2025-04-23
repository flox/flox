{
  overlay = final: prev: {
    nef =
      final.makeScope (scope: final.callPackageWith ({ lib = final; } // final // scope))
        (self: {
          dirToAttrs = (self.callPackage ./dirToAttrs.nix { }).dirToAttrs;
        });
  };
}
