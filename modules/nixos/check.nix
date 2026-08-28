# Evaluation-only smoke test for the Flox NixOS module.
#
# Exercises both configuration methods on a sample system and forces the
# generated units and the module's option documentation. This catches
# evaluation regressions (e.g. functions removed from the nixpkgs lib, or
# module options drifting from the units that consume them) without
# building anything, so it runs on any system.
{
  pkgs,
  nixpkgs,
  module,
}:

let
  inherit (nixpkgs) lib;

  sys = lib.nixosSystem {
    system = "x86_64-linux";
    modules = [
      module
      {
        system.stateVersion = "25.05";
        services.flox = {
          enable = true;
          activations.sample = {
            environment = "flox/sample";
            floxHubTokenFile = "/run/keys/sample.token";
            autoPull.enable = true;
            autoRestart.enable = true;
          };
        };
        systemd.services.sample-override = {
          flox = {
            environment = "flox/sample";
            execStart = "sample --flag";
            autoPull.enable = true;
          };
        };
        # Exercises the makeJobScript path, which the execStart sample
        # above leaves unevaluated.
        systemd.services.sample-script.flox = {
          environment = "flox/sample";
          script = "sample --flag";
        };
      }
    ];
  };

  unitTexts = map (unit: sys.config.systemd.units.${unit}.text) [
    "sample.service"
    "flox-pull@.service"
    "flox-autopull@.service"
    "flox-autopull@sample.timer"
    "sample-override.service"
    "flox-autopull@sample-override.timer"
    "sample-script.service"
  ];

  # The NixOS manual renders all option descriptions on real systems, so
  # force them here as well.
  descriptionsOf =
    options:
    map (doc: doc.description) (
      lib.filter (doc: (doc.description or null) != null) (lib.optionAttrSetToDocList options)
    );
  serviceOptionDocs = descriptionsOf sys.options.services.flox;
  overridesOptionDocs = descriptionsOf (sys.options.systemd.services.type.getSubOptions [ ]).flox;

  # With the services subsystem disabled nothing is added to the system:
  # the `programs.flox` half only installs the CLI and sets substituters.
  disabledSys = lib.nixosSystem {
    system = "x86_64-linux";
    modules = [
      module
      {
        system.stateVersion = "25.05";
        systemd.services.sample-override.flox = {
          environment = "flox/sample";
          execStart = "sample --flag";
        };
      }
    ];
  };
  disabledUnits = lib.filter (lib.hasPrefix "flox-") (lib.attrNames disabledSys.config.systemd.units);
  disabledTmpfiles = lib.filter (lib.hasInfix "flox") disabledSys.config.systemd.tmpfiles.rules;

  # An override configured without `services.flox.enable` is reported
  # rather than left to fail at runtime on a missing flox-pull@ unit.
  disabledAssertions = lib.filter (
    assertion: !assertion.assertion && lib.hasInfix "services.flox.enable" assertion.message
  ) disabledSys.config.assertions;

  moduleAssertions = lib.filter (
    assertion: !assertion.assertion && lib.hasInfix "flox" assertion.message
  ) sys.config.assertions;

in
assert moduleAssertions == [ ];
assert disabledUnits == [ ];
assert disabledTmpfiles == [ ];
assert lib.length disabledAssertions == 1;
builtins.deepSeq (unitTexts ++ serviceOptionDocs ++ overridesOptionDocs) (
  pkgs.writeText "flox-nixos-module-check" "ok"
)
