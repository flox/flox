{
  description = "A very basic flake";
  inputs.capacitor.url = "github:flox/capacitor/v0";

  # Flake to analyze
  # The analysis will access selected `__reflect` data from the target
  # using `placeholder` as `apps` and `lib` should be accessible without a target
  # TODO: can we now do `.follows = "/"` ?
  inputs.target.url = "path:./placeholder";

  outputs = {
    capacitor,
    target,
    ...
  } @ args:
    capacitor args (context: {
      apps.analysis = {};

      config.plugins = [
        # mount the analysis attributes
        (import ./plugins/eval.nix context {
          inherit target;
          attributePath = ["analysis" "eval"];
        })
        (import ./plugins/eval.nix context {
          inherit target;
          attributePath = ["analysis" "build"];
          build = {analyzeOutput = true;};
        })

        # load and expose libraries and apps
        (capacitor.plugins.localResources {type = "lib";})
        capacitor.plugins.importers.lib
      ];

      # Experimental
      passthru.bundlers.aarch64-darwin = {
        default = drv: capacitor.inputs.nixpkgs.legacyPackages.aarch64-darwin.writeText "${drv.name}-analysis.json" (builtins.toJSON (context.self.lib.readPackage {} {} drv));
      };
    });
}
