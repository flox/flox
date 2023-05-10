# capacitor context
{
  self,
  lib,
  inputs,
}:
# First argument:
# Externally determined metadata
{
  attrPath ? [],
  namespace ? [],
  flakeRef ? null, # "self" gets special treatment
}:
# Second argument
# enable (implicit) building
{
  analyzeOutput ? substituteOnly,
  # if true uses builtins.storePath which does not attempts to build
  # requires `--impure`
  substituteOnly ? false,
  ...
} @ buildOptions: drv: let
  inherit (self.lib) inspectBuild;

  element = rec {
    active = true;
    inherit attrPath;
    # TODO deduplicate with logic for floxEnvs
    # normalize to include "flake:", which is included in manifest.json
    originalUrl =
      if flakeRef == "self"
      then "." # TODO: use outPath?
      else
        (
          if flakeRef == null || builtins.match ".*:.*" flakeRef == []
          then flakeRef
          else "flake:${flakeRef}"
        );
    # TODO deduplicate with logic for floxEnvs and figure out a better way to
    # store flake resolution information
    url =
      if flakeRef == "self"
      then ""
      else if flakeRef != null
      then let
        flake =
          builtins.getFlake flakeRef;
        # this assumes that either flakeRef is not indirect, or if it is indirect, the flake it
        # resolves to contains a branch
      in "${originalUrl}/${flake.rev}"
      # TODO this violates the catalog schema, so it must be set with
      # postprocessing
      else null;
    storePaths =
      if drv.meta ? outputsToInstall
      then
        # only include outputsToInstall
        (builtins.map (outputName: eval.outputs.${outputName})
          drv.meta.outputsToInstall)
      else lib.attrValues eval.outputs;
  };

  eval = {
    # flake.locked = builtins.removeAttrs inputs.target.sourceInfo ["outPath"];
    inherit (drv) name system meta;
    inherit attrPath namespace;
    drvPath = builtins.unsafeDiscardStringContext drv.drvPath;
    pname = (builtins.parseDrvName drv.name).name;
    version =
      if (builtins.parseDrvName drv.name).version != ""
      then (builtins.parseDrvName drv.name).version
      else if drv ? version && drv.version != "" && drv.version != null
      then drv.version
      else "unknown";
    outputs = let
      outputs = drv.outputs or ["out"];
    in
      lib.genAttrs outputs (output: builtins.unsafeDiscardStringContext drv.${output}.outPath);
  };
in {
  inherit element eval;
  build = lib.optional analyzeOutput (inspectBuild buildOptions drv.outPath);
  version = 1;
}
