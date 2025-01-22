# A wrapper around dockerTools.streamLayeredImage that
# composes a storePath to an environment with a shell and core utils
{
  # the (bundled) nixpkgs flake
  nixpkgsFlakeRef,
  # the path to the environment that was built previously
  environmentOutPath,
  # the system to build for
  system,
  containerSystem,
  environment ? builtins.storePath environmentOutPath,
  nixpkgsFlake ? builtins.getFlake nixpkgsFlakeRef,
  pkgs ? nixpkgsFlake.legacyPackages.${system},
  containerPkgs ? nixpkgsFlake.legacyPackages.${containerSystem},
  containerName ? "flox-env-container",
  containerTag ? null,
  containerCreated ? "now",
  containerConfigJSON ? "{}",
}:
let
  inherit (builtins)
    fromJSON
    toString
    elemAt
    match
    ;
  inherit (pkgs.lib)
    mapAttrsToList
    optionalString
    optionalAttrs
    optionals
    toIntBase10
    assertMsg
    isValidPosixName
    isInt
    ;
  inherit (pkgs.lib.meta)
    lowPrio
    ;

  containerConfig = fromJSON containerConfigJSON;

  nixStoreOwner = (containerConfig.User or "0:0");

  isNixStoreUserOwned = (null == (match "^(root|0):\?(root|0)\?$" nixStoreOwner));

  nixStoreUserGroup =
    let
      userGroupValues =
        let
          values = match "^(([_]*[[:alpha:]]+):?|([[:digit:]]+):?)(([_]*[[:alpha:]]+)|([[:digit:]]+))?$" nixStoreOwner;
        in
        assert assertMsg (
          null != values
        ) "Failed to parse nixStoreOwner, ${nixStoreOwner} did not match the expected pattern";
        values;
      uname = elemAt userGroupValues 1;
      gname = if (null != (elemAt userGroupValues 4)) then (elemAt userGroupValues 4) else "flox";
      uid =
        if (null != (elemAt userGroupValues 2)) then toIntBase10 (elemAt userGroupValues 2) else 10000;
      gid =
        if (null != (elemAt userGroupValues 5)) then toIntBase10 (elemAt userGroupValues 5) else 10000;
    in
    assert assertMsg (null != uname && null != uid) "Either uname or uid must always be set";
    assert assertMsg (
      null != gname
    ) "The group part should not be null, if left empty it must get swallowed";
    assert assertMsg (isValidPosixName uname) "uname must be a valid POSIX name";
    assert assertMsg (isValidPosixName gname) "gname must be a valid POSIX name";
    assert assertMsg (isInt uid) "uid must be an integer";
    assert assertMsg (isInt gid) "gid must be an integer";
    {
      inherit
        uname
        gname
        uid
        gid
        ;
    };

  fakeNss = containerPkgs.dockerTools.fakeNss.override {
    extraPasswdLines = optionals isNixStoreUserOwned [
      "${nixStoreUserGroup.uname}:x:${toString nixStoreUserGroup.uid}:${toString nixStoreUserGroup.gid}:created by Flox:/var/empty:/bin/sh"
    ];
    extraGroupLines = optionals isNixStoreUserOwned [
      "${nixStoreUserGroup.gname}:x:${toString nixStoreUserGroup.gid}:"
    ];
  };

  buildLayeredImageArgs =
    optionalAttrs (isNixStoreUserOwned) {
      inherit (nixStoreUserGroup)
        uname
        gname
        uid
        gid
        ;

      # chown the /run directory to the nixStoreOwner, so that Nix can run as a
      # single user installation inside the container
      fakeRootCommands = ''
        chown -R ${toString nixStoreUserGroup.uid}:${toString nixStoreUserGroup.gid} /run
      '';
      enableFakechroot = true;
    }
    // {
      name = containerName;
      tag = containerTag;
      created = containerCreated;

      # Ensures the container configuration contains the correct architecture of
      # the binaries contained within it. Runtimes can use this to short-circuit
      # errors when users try to run a container on an incompatible architecture.
      architecture = containerPkgs.go.GOARCH;

      # Activate script requires writable directory, /run feels like a logical place.
      extraCommands = ''
        mkdir -m 1770 run
        mkdir -p -m 1770 run/flox
      '';

      # symlinkJoin fails when drv contains a symlinked bin directory, so wrap in an additional buildEnv.
      contents = pkgs.buildEnv {
        name = "contents";
        paths = [
          fakeNss
          environment
          (lowPrio containerPkgs.bashInteractive) # for a usable shell
          (lowPrio containerPkgs.coreutils) # for just the basic utils
        ];
      };
      config = containerConfig // {
        # Use activate script as the [one] entrypoint capable of
        # detecting interactive vs. command activation modes.
        # Usage:
        #   podman run -it
        #     -> launches interactive shell with controlling terminal
        #   podman run -i <cmd>
        #     -> invokes interactive command
        #   podman run -i [SIC]
        #     -> launches crippled interactive shell with no controlling
        #        terminal .. kinda useless
        Entrypoint = [ "${environment}/activate" ];

        Env = mapAttrsToList (name: value: "${name}=${value}") {
          "FLOX_ENV" = environment;
          "FLOX_PROMPT_ENVIRONMENTS" = "floxenv";
          "FLOX_PROMPT_COLOR_1" = "99";
          "FLOX_PROMPT_COLOR_2" = "141";
          "_FLOX_ACTIVE_ENVIRONMENTS" = "[]";
          "FLOX_SOURCED_FROM_SHELL_RC" = "1"; # don't source from shell rc (again)
          "_FLOX_FORCE_INTERACTIVE" = "1"; # Required when running podman without "-t"
          "FLOX_SHELL" = "${containerPkgs.bashInteractive}/bin/bash";
          "FLOX_RUNTIME_DIR" = "/run/flox";
        };
      };
    };
in
pkgs.dockerTools.streamLayeredImage buildLayeredImageArgs
