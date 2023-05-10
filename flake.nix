{
  description = "Floxpkgs/Project Template";

  inputs.capacitor.url = "github:flox/capacitor?ref=v0";
  inputs.nixpkgs-flox.url = "github:flox/nixpkgs-flox";
  inputs.nixpkgs-flox.inputs.flox.follows = "/";

  # Declaration of external resources
  # =================================
  inputs.shellHooks.url = "github:cachix/pre-commit-hooks.nix";
  # =================================

  outputs = args @ {
    capacitor,
    nixpkgs-flox,
    ...
  }:
    capacitor args ({
      self,
      lib,
      ...
    }: let
      defaultPlugins = [
        (capacitor.plugins.allLocalResources {})
        (import ./capacitor-plugins/catalog.nix {inherit self lib;} {})
        (import ./capacitor-plugins/floxEnvs.nix {inherit self lib;} {
          sourceType = "packages";
          dir = "pkgs";
        })
        (import ./capacitor-plugins/rootFloxEnvs.nix {inherit self lib;} {})
      ];
    in {
      config = {
        extraPlugins =
          defaultPlugins
          ++ [
            (capacitor.plugins.plugins {dir = ./capacitor-plugins;})
            (capacitor.plugins.templates {})
          ];
      };

      passthru.capacitor = capacitor;

      passtrhu.defaultPlugins = defaultPlugins;

      passthru.project = args: config:
        capacitor ({nixpkgs = nixpkgs-flox;} // args) (
          context:
            lib.recursiveUpdate {
              config.plugins = capacitor.defaultPlugins ++ defaultPlugins;
            }
            (config context)
        );
    });
}
