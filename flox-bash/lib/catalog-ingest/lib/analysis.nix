{
  self,
  lib,
  nixpkgs,
}: {target}: let
  inherit (self.lib) readPackage isValidDrv;
  inherit (lib.capacitor.capacitate) materialize;
  inherit (lib.capacitor.utils) mapAttrsRecursiveCondPath;

  analysisMapper = {
    flatten,
    buildOptions,
  }: {
    isCapacitated, # true if capacitated
    # the attribute path of the definition without system
    # packages.flox -> ["flox"]
    namespace,
    # which flake'd definition (first part of attribute path in legacyPackages)
    flakePath,
    # the derivation
    system,
    # TODO(capacitor) encode outputType
    # or
    # TODO(flox) if going with bundlers, use and fixup ./readPackage.nix output
    outputType ? "packages",
    ...
  }: let
    attrPath = lib.flatten [outputType system namespace];
  in {
    value =
      readPackage {
        inherit attrPath namespace;
      }
      buildOptions
      (lib.getAttrFromPath namespace target.packages.${system});
    path = attrPath;
    use = !flatten || isCapacitated;
  };

  # Capacitor integration
  # Generates the structure mounted at `analysis`
  # Applies the above mapper to every package
  analysisGen = buildOptions: generated: let
    materialize' = flatten: materialize (analysisMapper {inherit flatten buildOptions;});

    joinProjects = self': let
      packages = materialize' false self';

      capacitated =
        lib.foldl' (a: b: a // b)
        (materialize' true self');

      self = lib.attrValues capacitated;

      derivations = lib.genAttrs ["x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin"] (
        system: let
          pkgs = nixpkgs.legacyPackages.${system};
          topLevel =
            lib.mapAttrs
            (name: value: pkgs.writeText "${name}_reflection.json" (builtins.toJSON value))
            {inherit self packages;};

          individual =
            lib.mapAttrsRecursiveCond (v:
              !(
                builtins.length (builtins.attrNames v)
                == 3
                && builtins.all (a: lib.hasAttr a v) ["eval" "build" "element"]
              ))
            (path: value: pkgs.writeText "${value.eval.pname}_reflection.json" (builtins.toJSON value))
            packages;
        in
          topLevel // {packages = topLevel.packages // {passthru = individual;};}
      );
    in
      packages;
  in
    joinProjects generated;
in {inherit analysisGen;}
