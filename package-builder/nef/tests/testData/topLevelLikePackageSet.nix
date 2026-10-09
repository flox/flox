{ lib }:

# Shaped like top-level nixpkgs: `newScope` closes over the final
# fixpoint, as in `pkgs/top-level/splice.nix:123`. `basePackageSet.nix`
# is a `lib.makeScope` set, which hides that and has no `.extend`.
lib.makeExtensible (self: {
  newScope = extra: lib.callPackageWith (self // extra);

  zlib = "base zlib";
  hello = throw "This will be overridden";

  # Targets for deep-override tests, which can only replace existing
  # attributes.
  topLevelDependency = throw "This will be overridden";
  catalogSeeker = throw "This will be overridden";

  # No `newScope` or `callPackage`, so its packages use `callPackageIn`'s
  # fallback branch.
  nested = lib.makeExtensible (_final: {
    catalogSeeker = throw "This will be overridden";
  });
})
