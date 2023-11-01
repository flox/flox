# ============================================================================ #
#
# Create an environment from a `lockfile.json` file.
#
# ---------------------------------------------------------------------------- #
{
  lockfilePath ?
    throw
    "flox: You must provide the path to a lockfile.",
  system ? builtins.currentSystem or "unknown",
  ...
}: let
  lockfileContents = builtins.fromJSON (builtins.readFile lockfilePath);

  # Convert manifest elements to derivations.
  # Return `[]' for non-active elements.
  tryGetDrv = system: package: let
    attrs = builtins.getFlake package.${system}.url;
    drv = builtins.foldl' (pathComponent: flakeAttrs: builtins.getAttr flakeAttrs pathComponent) attrs package.${system}.path;
    toInstall = drv.meta.outputsToInstall or drv.outputs;
    numOutputs = builtins.length toInstall;
    priority = drv.meta.priority or 5;
  in
    # assert drv?outPath;
    if package.${system} == null
    then []
    else ["true" priority numOutputs] ++ map (output: builtins.getAttr output drv) toInstall;
in
  derivation {
    inherit system;
    name = "flox-env";
    builder = "builtin:buildenv";
    manifest = "/dummy";
    derivations = builtins.concatMap (tryGetDrv system) (builtins.attrValues lockfileContents.packages);
  }
# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #

