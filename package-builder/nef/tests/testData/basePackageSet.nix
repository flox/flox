{ lib }:

lib.makeScope lib.callPackageWith (self: {
  topLevelValue = "value";
  topLevelDependency = throw "This will be overriden";
  topLevelDependent = self.callPackage (
    { topLevelValue, topLevelDependency }: "depends on ${topLevelValue} and ${topLevelDependency}"
  ) { };

  # Sets created wit make scope with attributes depending
  # on ambient ones.
  setMakeExtensible = lib.makeExtensible (final: {
    extensibleValue = "value";
    extensibleDependency = throw "This will be overriden";
    extensibleDependent = "depends on ${final.extensibleValue}, ${self.topLevelValue} and ${final.extensibleDependency}";
  });

  # Sets created wit make scope with attributes depending
  # on both higher level dependencies and ambient ones.
  setMakeScope = lib.makeScope self.newScope (self: {
    makeScopeValue = "value";
    makeScopeDependency = throw "This will be overriden";
    makeScopeDependent = self.callPackage (
      {
        makeScopeValue,
        topLevelDependency,
        makeScopeDependency,
      }:
      "depends on ${makeScopeValue}, ${topLevelDependency} and ${makeScopeDependency}"
    ) { };
  });

  setNotExtendable = { };
})
