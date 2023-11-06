# Build an environment from a collection of packages
{
  lockfilePath ?
    throw
    "flox: You must provide the path to a lockfile.",
  system ? builtins.currentSystem or "unknown",
  ...
}: let
  lockfileContents = builtins.fromJSON (builtins.readFile lockfilePath);
  nixpkgsFlake = builtins.getFlake lockfileContents.registry.inputs.nixpkgs.url;
  pkgs = nixpkgsFlake.legacyPackages.${system};
  # Convert manifest elements to derivations.
  tryGetDrv = system: package: let
    flake = builtins.getFlake package.${system}.url;
    drv = builtins.foldl' (attrs: pathComponent: builtins.getAttr pathComponent attrs) flake package.${system}.path;
  in
    if builtins.isNull package.${system}
    then null
    else drv;
  activateScript = pkgs.writeTextFile {
    name = "activate";
    executable = true;
    destination = "/activate";
    # TODO don't hardcode 0100_common-paths.sh
    text = ''
      . ${./set-prompt.sh}
      . ${./profile.d/0100_common-paths.sh}
      . ${./source-profiles.sh}
    '';
  };
  entries =
    builtins.filter
    (p: !builtins.isNull p)
    (builtins.map (tryGetDrv system)
      (builtins.attrValues lockfileContents.packages));
in
  pkgs.symlinkJoin {
    name = "flox-env";
    paths =
      entries
      ++ [activateScript];
  }
