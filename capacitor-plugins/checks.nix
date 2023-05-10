{lib, ...}: {}: let
  materialize = lib.capacitor.capacitate.materialize;

  testsMapper = context: {
    namespace,
    flakePath,
    system,
    fn,
    ...
  }: let
    value = (context.context' system).callPackageWith {} fn {};
    flatAttrPath = lib.showAttrPath (lib.flatten [namespace]);
    checkedOn = (lib.attrByPath ["meta" "checkOn"] [] value) ++ ["local"];
  in
    map (runner: {
      path = [system "__checkOn" runner flatAttrPath]; # flatten attrpath
      value = value;
    })
    checkedOn;
in
  {context, ...}: let
    generated = context.closures "checks";
  in {
    checks = materialize (testsMapper context) generated;
  }
