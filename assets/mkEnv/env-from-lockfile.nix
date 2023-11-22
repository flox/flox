# Build an environment from a collection of packages
{
  lockfilePath ? throw "flox: You must provide the path to a lockfile.",
  system ? builtins.currentSystem or "unknown",
  ...
}: let
  lockfileContents = builtins.fromJSON (builtins.readFile lockfilePath);
  nixpkgsFlake = with lockfileContents.registry.inputs.nixpkgs.from; builtins.getFlake "${type}:${owner}/${repo}/${rev}";
  pkgs = nixpkgsFlake.legacyPackages.${system};
  lib = nixpkgsFlake.lib;

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

  profiledScripts = pkgs.runCommand "flox-profile.d-scripts" {} ''
    mkdir -p $out/etc/profile.d
    cp -R ${./profile.d}/* $out/etc/profile.d/
  '';

  activateScript = pkgs.writeTextFile {
    name = "flox-activate";
    executable = true;
    destination = "/activate";
    text = ''
      # We use --rcfile to activate using bash which skips sourcing ~/.bashrc,
      # so source that here.
      if [ -f ~/.bashrc ]
      then
          source ~/.bashrc
      fi

      . ${./set-prompt.sh}
      . ${./source-profiles.sh}

      ${lib.optionalString (lockfileContents ? manifest.hook.script) ''
        ${lockfileContents.manifest.hook.script}
      ''}
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
