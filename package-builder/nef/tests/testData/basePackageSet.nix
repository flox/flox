{ lib }:

lib.makeScope lib.callPackageWith (self: {
  topLevelValue = "value";
  topLevelDependency = throw "This will be overridden";
  topLevelDependent = self.callPackage (
    { topLevelValue, topLevelDependency }: "depends on ${topLevelValue} and ${topLevelDependency}"
  ) { };

  # Sets created wit make scope with attributes depending
  # on ambient ones.
  setMakeExtensible = lib.makeExtensible (final: {
    extensibleValue = "value";
    extensibleDependency = throw "This will be overridden";
    extensibleDependent = "depends on ${final.extensibleValue}, ${self.topLevelValue} and ${final.extensibleDependency}";
  });

  # Sets created wit make scope with attributes depending
  # on both higher level dependencies and ambient ones.
  setMakeScope = lib.makeScope self.newScope (self: {
    makeScopeValue = "value";
    makeScopeDependency = throw "This will be overridden";
    makeScopeDependent = self.callPackage (
      {
        makeScopeValue,
        topLevelDependency,
        makeScopeDependency,
      }:
      "depends on ${makeScopeValue}, ${topLevelDependency} and ${makeScopeDependency}"
    ) { };

    # Targets for nested deep-override tests, which can only replace
    # existing attributes.
    wantsExistingNestedSibling = throw "This will be overridden";
    wantsMissingNestedSibling = throw "This will be overridden";
  });

  setNotExtendable = { };

  # Targets for deep-override tests, which can only replace existing
  # attributes. No test reads these values.
  usesLocalTool = throw "This will be overridden";
  reachesProjectPkg = throw "This will be overridden";
  overrideA = throw "This will be overridden";
  overrideB = throw "This will be overridden";
  usesTopLevelDependency = throw "This will be overridden";
  wantsTopLevelSibling = throw "This will be overridden";
  wantsOwnSibling = throw "This will be overridden";
  wantsOtherSourceSibling = throw "This will be overridden";
  wantsOtherOwnSibling = throw "This will be overridden";
})
