# A wrapper around dockerTools.streamLayeredImage that
# composes a storePath to an environment with a shell and core utils
{
  # the (bundled) nixpkgs flake
  nixpkgsFlakeRef,
  # the path to the environment that was built previously
  environmentOutPath,
  interpreterPath,
  # what mode it should be activation with
  activationMode,
  # the system to build for
  system,
  containerSystem,
  # Optional: store path to the flox binary built for containerSystem.
  # When set, flox is added to the guest image so commands like `flox
  # list` work inside the container. The bash shim is omitted because
  # the real binary handles all subcommands including `flox deactivate`.
  floxBin ? "",
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
    storePath
    ;
  inherit (pkgs.lib)
    optionalAttrs
    optionals
    optionalString
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

  workingDir = (containerConfig.WorkingDir or null);

  isNixStoreUserOwnedRegex = "^(root|0):\?(root|0)\?$";

  unameGnameRegex = "^(([_]*[[:alpha:]]+):?|([[:digit:]]+):?)(([_]*[[:alpha:]]+)|([[:digit:]]+))?$";

  isNixStoreUserOwned = (null == (match isNixStoreUserOwnedRegex nixStoreOwner));

  mkUnameGnameUidGid =
    userGroup:
    let
      userGroupValues =
        let
          values = match unameGnameRegex userGroup;
        in
        assert assertMsg (
          null != values
        ) "Failed to parse containerize.config.User, ${userGroup} did not match the expected pattern";
        values;
      uname = if (null != (elemAt userGroupValues 1)) then (elemAt userGroupValues 1) else "flox";
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

  nixStoreUserGroup = mkUnameGnameUidGid nixStoreOwner;

  fakeNss = containerPkgs.dockerTools.fakeNss.override {
    extraPasswdLines = optionals isNixStoreUserOwned [
      "${nixStoreUserGroup.uname}:x:${toString nixStoreUserGroup.uid}:${toString nixStoreUserGroup.gid}:created by Flox:/var/empty:/bin/sh"
    ];
    extraGroupLines = optionals isNixStoreUserOwned [
      "${nixStoreUserGroup.gname}:x:${toString nixStoreUserGroup.gid}:"
    ];
  };

  # Whether a real flox binary is being included in the image.
  # When true: real binary is available, skip the deactivate-only shim.
  # When false: no flox in image, shim keeps `flox deactivate` working.
  hasFloxBin = floxBin != "";

  # For field definitions, see `ActivateCtx` in `flox-core`
  activateCtx = {
    mode = "${activationMode}";
    shell = {
      bash = "${containerPkgs.bash}/bin/bash";
    };
    invocation_type = null;
    remove_after_reading = false;
    # When a real flox binary is present the prompt hook is meaningful
    # (it calls back into flox for auto-activation). When no flox binary
    # is present, disable_hook avoids the "command not found" error that
    # would occur when bash tries to invoke an empty flox_bin.
    disable_hook = !hasFloxBin;
    # flox_bin is read by flox-activations to generate the hook code and
    # to decide whether to emit the deactivate-only shim. An empty string
    # means "no real flox binary present".
    flox_bin = optionalString hasFloxBin "${storePath floxBin}/bin/flox";
    flox_activate_store_path = "${environment}";
    activation_state_dir = "/run/flox/container-activations/${baseNameOf environment}";
    attach_ctx = {
      env = "${environment}"; # FIXME: Incorrect for containers.
      env_description = "${containerName}";
      env_cache = "/tmp";
      prompt_color_1 = "99";
      prompt_color_2 = "141";
      interpreter_path = "${interpreterPath}";
      flox_prompt_environments = "${containerName}";
      set_prompt = true;
      flox_env_cuda_detection = "0";
      flox_active_environments = "[]";
    };
    project_ctx = null;
  };

  activateCtxJson = builtins.toJSON activateCtx;
  activateCtxStorePath = pkgs.writeTextFile {
    name = "activations-context";
    text = activateCtxJson;
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
      fakeRootCommands =
        ''
          chown -R ${toString nixStoreUserGroup.uid}:${toString nixStoreUserGroup.gid} /run
        ''
        + optionalString (workingDir != null) ''
          mkdir -p -m 0755 "${workingDir}"
          chown ${toString nixStoreUserGroup.uid}:${toString nixStoreUserGroup.gid} "${workingDir}"
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

      # No /tmp by default: https://github.com/NixOS/nixpkgs/issues/257172
      # Activate script requires writable directory, /run feels like a logical place.
      # /home/flox gives the real flox binary a writable HOME for config and
      # state files (XDG_CONFIG_HOME, XDG_STATE_HOME, XDG_CACHE_HOME, and
      # XDG_RUNTIME_DIR are all routed there via the activation context).
      extraCommands = ''
        mkdir -m 1777 tmp
        mkdir -m 1770 run
        mkdir -p -m 1770 run/flox
        mkdir -p -m 0700 home/flox
      '';

      # symlinkJoin fails when drv contains a symlinked bin directory, so wrap in an additional buildEnv.
      contents = pkgs.buildEnv {
        name = "contents";
        paths =
          [
            fakeNss
            environment
            (lowPrio containerPkgs.bash) # for a usable shell
            (lowPrio containerPkgs.coreutils) # for just the basic utils
          ]
          # Include the real flox binary when provided so guest commands
          # like `flox list` work against the bind-mounted project lockfile.
          ++ optionals hasFloxBin [
            (lowPrio (storePath floxBin))
          ];
      };
      config =
        containerConfig
        // {
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
          Entrypoint = [
            "${environment}/libexec/flox-activations"
            "activate"
            "--activate-data"
            "${activateCtxStorePath}"
          ];
        }
        // optionalAttrs hasFloxBin {
          # Point flox at writable per-container directories so it can
          # store config, state, and runtime files. The container's /tmp
          # and /home/flox are the only writable locations in the image.
          Env = [
            "HOME=/home/flox"
            "XDG_CONFIG_HOME=/home/flox/.config"
            "XDG_STATE_HOME=/home/flox/.local/state"
            "XDG_CACHE_HOME=/home/flox/.cache"
            "XDG_RUNTIME_DIR=/run/flox/runtime"
          ];
        };

      passthru = {
        # These tests can be run with the following command from the root of the repository:
        #     $ nix eval --impure --expr '(import ./mkContainer/mkContainer.nix { nixpkgsFlakeRef = "github:nixos/nixpkgs?ref=nixos-24.11"; environmentOutPath = null; system = builtins.currentSystem; containerSystem = builtins.currentSystem; }).passthru.tests'
        #     $ [ ]
        # If it returns anything other than [ ], then the tests failed.
        tests = import ./tests.nix {
          lib = pkgs.lib;
          internals = {
            inherit isNixStoreUserOwnedRegex unameGnameRegex mkUnameGnameUidGid;
          };
        };
      };
    };
in
pkgs.dockerTools.streamLayeredImage buildLayeredImageArgs
