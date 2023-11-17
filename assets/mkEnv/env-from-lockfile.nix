# Build an environment from a collection of packages
{
  lockfilePath ? throw "flox: You must provide the path to a lockfile.",
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

  mkEnv = ./.;

  profiledScripts = pkgs.runCommand "flox-profile.d-scripts" {} ''
    mkdir -p $out/etc/profile.d
    cp -R ${mkEnv}/profile.d/* $out/etc/profile.d/
  '';

  activateScript = pkgs.writeTextFile {
    name = "flox-activate";
    executable = true;
    destination = "/activate";
    text = ''
      . ${mkEnv}/set-prompt.sh
      . ${mkEnv}/source-profiles.sh
    '';
  };
in
  pkgs.symlinkJoin {
    name = "flox-env";
    paths =
      entries
      ++ [
        profiledScripts
        activateScript
      ];
  }
