{ lib }:
let
  baseDir = "${./testData}";
  reflect = lib.nef.reflect;
  collect = test: reflect.collectAttrPaths (lib.nef.dirToAttrs "${baseDir}/${test}");
in

{
  "test: reflect make targets for toplevel packages" = {
    expr = reflect.makeTargets (collect "simple");
    expected = "a b";
  };

  "test: reflect make targets for nested packages" = {
    expr = reflect.makeTargets (collect "nested");
    expected = "nestedPkgs.nestedPkg nestedPkgs.otherNestedPkg toplevelPkg";
  };

  # while correct, these kind of quoting tends to casuse issues
  # on the fringe between make and the cli.
  # Especially <space>s may be tricky to pass as make arguments,
  # but even mere quoting sets off the cli.
  # We will likely address those isseus in a future issue
  # that asks for sanitation of make target names.
  # For now we will just document the behavior:

  "test: reflect make targets for nested packages, quotes names that look like attrPath" = {
    expr = reflect.makeTargets (collect "nestedWithFaux");
    expected = "nestedPkgs.real \"nestedPkgs.faux\" toplevelPkg";
  };

  "test: reflect quotes attrPaths with special characters" = {
    expr = reflect.makeTargets (collect "specialCharacters");
    expected = "\"@at\" \"libc++\" \"with space\"";
  };

  "test: collect includes paths" = {
    expr = collect "nested";
    expected = [
      {
        attrPath = [
          "nestedPkgs"
          "nestedPkg"
        ];
        attrPathStr = "nestedPkgs.nestedPkg";
        absFilePath = "${baseDir}/nested/nestedPkgs/nestedPkg.nix";
        relFilePath = "nestedPkgs/nestedPkg.nix";
      }
      {
        attrPath = [
          "nestedPkgs"
          "otherNestedPkg"
        ];
        attrPathStr = "nestedPkgs.otherNestedPkg";
        absFilePath = "${baseDir}/nested/nestedPkgs/otherNestedPkg/default.nix";
        relFilePath = "nestedPkgs/otherNestedPkg/default.nix";
      }
      {
        attrPath = [ "toplevelPkg" ];
        attrPathStr = "toplevelPkg";
        absFilePath = "${baseDir}/nested/toplevelPkg.nix";
        relFilePath = "toplevelPkg.nix";
      }
    ];
  };

}
