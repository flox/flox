{ lib }:
let
  instantiate = lib.nef.instantiate;

  base = import ./testData/basePackageSet.nix { inherit lib; };

  overlayFor = overridesClosure: instantiate.mkGlobalOverridesOverlay { inherit overridesClosure; };

  # The fixture base is a `makeScope` set (like `python3Packages`), so it
  # only has `.overrideScope`; real nixpkgs also has `.extend`, which
  # `instantiateFromSourceInfo` calls directly. Alias the two so this
  # fixture can stand in for `nixpkgs` in `consumerAgainst` below.
  overriddenBase =
    overridesClosure:
    let
      extended = base.overrideScope (overlayFor overridesClosure);
    in
    extended // { extend = extended.overrideScope; };

  # A "catalog package" instantiated the same way `instantiateFromSourceInfo`
  # instantiates any project source, built against a given (already
  # overridden) base. `outPath` names the fixture directly, so this needs no
  # fetch of its own -- only the override's own source goes through
  # `fetchSource` inside `mkGlobalOverridesOverlay`.
  sourceAgainst =
    sourceDir: nixpkgs:
    instantiate.instantiateFromSourceInfo {
      inherit nixpkgs;
      sourceInfo = {
        outPath = sourceDir;
      };
      instantiatedCatalogsClosure = { };
    };

  consumerAgainst = sourceAgainst ./testData/globalOverrides/consumer;
  consumerShadowAgainst = sourceAgainst ./testData/globalOverrides/consumerShadow;
in
{
  "test: global override replaces the targeted attribute" = {
    expr =
      (overriddenBase {
        "repoA/topLevelDependency" = {
          attr_path = [ "topLevelDependency" ];
          source = {
            type = "path";
            path = ./testData/globalOverrides/repoA;
          };
        };
      }).topLevelDependency;
    expected = "overridden by repoA";
  };

  "test: global override reaches a catalog package's dependency closure" = {
    expr =
      (consumerAgainst (overriddenBase {
        "repoA/topLevelDependency" = {
          attr_path = [ "topLevelDependency" ];
          source = {
            type = "path";
            path = ./testData/globalOverrides/repoA;
          };
        };
      })).pkgs.usesOverride;
    expected = "consumer sees: overridden by repoA";
  };

  "test: global override does not see its own repository's pkgs/" = {
    expr =
      (overriddenBase {
        "repoD/probe" = {
          attr_path = [ "probe" ];
          source = {
            type = "path";
            path = ./testData/globalOverrides/repoD;
          };
        };
      }).probe;
    # Not "LEAKED FROM SIBLING PKGS" -- repoD/pkgs/topLevelValue.nix must
    # stay invisible to repoD's own override.
    expected = "saw: value";
  };

  "test: two global overrides targeting the same attr path throw naming both" = {
    expr =
      (overriddenBase {
        "repoA/topLevelDependency" = {
          attr_path = [ "topLevelDependency" ];
          source = {
            type = "path";
            path = ./testData/globalOverrides/repoA;
          };
        };
        "repoC/topLevelDependency" = {
          attr_path = [ "topLevelDependency" ];
          source = {
            type = "path";
            path = ./testData/globalOverrides/repoC;
          };
        };
      }).topLevelDependency;
    expectedError = {
      type = "ThrownError";
      msg = "Two global overrides target 'topLevelDependency': 'repoA/topLevelDependency' and 'repoC/topLevelDependency'.";
    };
  };

  # A colliding pair identical to the test above, but nothing here
  # forces `.topLevelDependency` -- only the unrelated `.topLevelValue`.
  # The collision is only detected while the overlay's tree is
  # assembled, which does have to happen just to build this attrset;
  # what must NOT happen is the throw firing at that point, before any
  # consumer forces the specific colliding attribute. Before the lazy
  # fix, this failed the same way the test above does, only for
  # `.topLevelValue`.
  "test: an unforced collision does not fail evaluation" = {
    expr =
      (overriddenBase {
        "repoA/topLevelDependency" = {
          attr_path = [ "topLevelDependency" ];
          source = {
            type = "path";
            path = ./testData/globalOverrides/repoA;
          };
        };
        "repoC/topLevelDependency" = {
          attr_path = [ "topLevelDependency" ];
          source = {
            type = "path";
            path = ./testData/globalOverrides/repoC;
          };
        };
      }).topLevelValue;
    expected = "value";
  };

  # A consumer with its own `pkgs/topLevelDependency.nix` must supersede
  # the colliding global override at that name: its definition doesn't
  # reference `prev`, so building it never reaches -- and never forces
  # the throw of -- the base's poisoned entry.
  "test: a consumer shadowing a collided name wins" = {
    expr =
      (consumerShadowAgainst (overriddenBase {
        "repoA/topLevelDependency" = {
          attr_path = [ "topLevelDependency" ];
          source = {
            type = "path";
            path = ./testData/globalOverrides/repoA;
          };
        };
        "repoC/topLevelDependency" = {
          attr_path = [ "topLevelDependency" ];
          source = {
            type = "path";
            path = ./testData/globalOverrides/repoC;
          };
        };
      })).pkgs.topLevelDependency;
    expected = "consumer's own topLevelDependency";
  };

  "test: a prefix collision throws naming both sources" = {
    expr =
      (overriddenBase {
        "repoA/topLevelDependency" = {
          attr_path = [ "topLevelDependency" ];
          source = {
            type = "path";
            path = ./testData/globalOverrides/repoA;
          };
        };
        "repoE/topLevelDependency.sub" = {
          attr_path = [
            "topLevelDependency"
            "sub"
          ];
          source = {
            type = "path";
            path = ./testData/globalOverrides/repoE;
          };
        };
      }).topLevelDependency;
    expectedError = {
      type = "ThrownError";
      msg = "Two global overrides target 'topLevelDependency': 'repoA/topLevelDependency' and 'repoE/topLevelDependency.sub'.";
    };
  };
}
