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
    self,
    capacitor,
    nixpkgs-flox,
    ...
  }: let
    inherit (capacitor) lib;

    defaultPlugins = [
      (capacitor.plugins.allLocalResources {})
      (import ./capacitor-plugins/catalog.nix {inherit self lib;} {})
      (import ./capacitor-plugins/floxEnvs.nix {inherit self lib;} {
        sourceType = "packages";
        dir = "pkgs";
      })
      (import ./capacitor-plugins/rootFloxEnvs.nix {inherit self lib;} {})
    ];

    project = args: config:
      capacitor ({nixpkgs = nixpkgs-flox;} // args) (
        context:
          lib.recursiveUpdate {
            config.plugins = capacitor.defaultPlugins ++ defaultPlugins;
          }
          (config context)
      );
  in
    project args (_: {
      config = {
        extraPlugins = [
          (capacitor.plugins.allLocalResources {})
          (import ./capacitor-plugins/catalog.nix {inherit self lib;} {})
          (capacitor.plugins.plugins {dir = ./capacitor-plugins;})
          (capacitor.plugins.templates {})
        ];
      };

      # reexport of capacitor
      passthru.capacitor = capacitor;
      # define default plugins
      passthru.defaultPlugins = defaultPlugins;
      # simple capacitor interface
      passthru.project = project;
    });
}
