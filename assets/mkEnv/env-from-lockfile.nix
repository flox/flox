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
  tryGetDrv = package: let
    flake = builtins.getFlake package.input.url;
    drv = builtins.foldl' (attrs: pathComponent: builtins.getAttr pathComponent attrs) flake package.attr-path;
  in
    if builtins.isNull package
    then null
    else drv;
  entries =
    builtins.filter
    (p: !builtins.isNull p)
    (builtins.map tryGetDrv
      (builtins.attrValues lockfileContents.packages.${system}));
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
in
  pkgs.symlinkJoin {
    name = "flox-env";
    paths =
      entries
      ++ [activateScript];
  }
