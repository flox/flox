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

  # Whether a real flox binary is being included in the image.
  # When true: real binary is available, skip the deactivate-only shim.
  # When false: no flox in image, shim keeps `flox deactivate` working.
  hasFloxBin = floxBin != "";

  # Passwd home directory for the container user. With a real flox binary
  # the guest needs a writable HOME for config/state/cache, so point passwd
  # at /home/flox to match the HOME env var and avoid a getpwuid-derived
  # lookup landing on the read-only /var/empty. Without flox, keep the
  # historic /var/empty.
  passwdHome = if hasFloxBin then "/home/flox" else "/var/empty";

  fakeNss = containerPkgs.dockerTools.fakeNss.override {
    extraPasswdLines = optionals isNixStoreUserOwned [
      "${nixStoreUserGroup.uname}:x:${toString nixStoreUserGroup.uid}:${toString nixStoreUserGroup.gid}:created by Flox:${passwdHome}:/bin/sh"
    ];
    extraGroupLines = optionals isNixStoreUserOwned [
      "${nixStoreUserGroup.gname}:x:${toString nixStoreUserGroup.gid}:"
    ];
  };

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
    # to decide whether to emit the deactivate-only shim. floxBin is the
    # package root, so bin/flox is appended here. An empty string means
    # "no real flox binary present".
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
      fakeRootCommands = ''
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
      # /home/flox is a writable HOME for a real guest flox; XDG_CONFIG_HOME,
      # XDG_STATE_HOME, and XDG_CACHE_HOME route under it. XDG_RUNTIME_DIR
      # routes to /run/flox/runtime, created here so the runtime dir exists.
      # /run/flox/log is writable by the guest user for process-compose logs;
      # the bind-mounted .flox/log is owned by the host uid and may not be
      # writable inside the container.
      extraCommands = ''
        mkdir -m 1777 tmp
        mkdir -m 1770 run
        mkdir -p -m 1770 run/flox
        mkdir -p -m 0700 run/flox/runtime
        mkdir -p -m 0700 run/flox/log
        mkdir -p -m 0700 home/flox
        # Resolve `localhost` in the guest so services bound to it (and
        # commands like `curl localhost:PORT`) work. fakeNss provides
        # passwd/group but not /etc/hosts.
        mkdir -p etc
        printf '127.0.0.1 localhost\n::1 localhost\n' > etc/hosts
      '';

      # symlinkJoin fails when drv contains a symlinked bin directory, so wrap in an additional buildEnv.
      contents = pkgs.buildEnv {
        name = "contents";
        paths = [
          fakeNss
          environment
          (lowPrio containerPkgs.bash) # for a usable shell
          (lowPrio containerPkgs.coreutils) # for just the basic utils
        ]
        # Include the real flox package root when provided so guest commands
        # like `flox list` work against the bind-mounted project lockfile.
        # No lowPrio: storePath yields a string (not a derivation, so lowPrio
        # would fail), and there is no bin/flox collision to deprioritize.
        ++ optionals hasFloxBin [
          (storePath floxBin)
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
          # store config, state, and runtime files. The container's /tmp,
          # /home/flox, and /run/flox/runtime are the writable locations
          # in the image. Append to any user-provided Env rather than
          # replacing it.
          #
          # _FLOX_SERVICES_SOCKET_OVERRIDE: pin the services socket to a fixed
          # guest path so both the activation entrypoint and in-guest
          # `flox services` always use the same socket without re-deriving a
          # path_hash inside the guest. /run/flox/runtime is already writable.
          # Path is 34 chars, well under the 107-char Linux sun_path limit.
          #
          # PROCESS_COMPOSE_BIN: flox-activations reads this at runtime to
          # locate the process-compose supervisor. The flox-activations binary
          # built for this image does not bake in PROCESS_COMPOSE_BIN at
          # compile time (unlike the flox binary), so we inject it here.
          Env = (containerConfig.Env or [ ]) ++ [
            "HOME=/home/flox"
            "XDG_CONFIG_HOME=/home/flox/.config"
            "XDG_STATE_HOME=/home/flox/.local/state"
            "XDG_CACHE_HOME=/home/flox/.cache"
            "XDG_RUNTIME_DIR=/run/flox/runtime"
            "_FLOX_SERVICES_SOCKET_OVERRIDE=/run/flox/runtime/services.sock"
            "PROCESS_COMPOSE_BIN=${containerPkgs.process-compose}/bin/process-compose"
          ];
        };

      passthru = {
        # These tests can be run with the following command from the root of the repository:
        #     $ nix eval --impure --expr '(import ./mkContainer/mkContainer.nix { nixpkgsFlakeRef = "github:nixos/nixpkgs?ref=nixos-24.11"; environmentOutPath = null; interpreterPath = "/interp"; activationMode = "dev"; system = builtins.currentSystem; containerSystem = builtins.currentSystem; }).passthru.tests'
        #     $ [ ]
        # If it returns anything other than [ ], then the tests failed.
        tests = import ./tests.nix {
          lib = pkgs.lib;
          internals = {
            inherit isNixStoreUserOwnedRegex unameGnameRegex mkUnameGnameUidGid;
          };
        };

        # Regression guard for the hasFloxBin=true branch: re-evaluate the
        # image with a real store-path root passed as floxBin and force the
        # derivation to instantiate (`drvPath` pulls in `contents` and
        # `config`, the bug sites). The `hello` package root stands in for
        # both the environment and the guest flox so the eval does not depend
        # on a real environmentOutPath. This is the branch that previously
        # broke on `builtins.storePath` (needs a package root, not a binary)
        # and `lowPrio` (needs a derivation, not a string). Evaluating
        # `.passthru.floxBinEval` throws on those errors without a full bake;
        # a clean run yields a `/nix/store/*.drv` path string.
        #     $ nix eval --impure --expr '(import ./mkContainer/mkContainer.nix { nixpkgsFlakeRef = "github:nixos/nixpkgs?ref=nixos-24.11"; environmentOutPath = null; interpreterPath = "/interp"; activationMode = "dev"; system = builtins.currentSystem; containerSystem = builtins.currentSystem; }).passthru.floxBinEval'
        #     $ "/nix/store/....drv"
        floxBinEval =
          (import ./mkContainer.nix {
            inherit
              nixpkgsFlakeRef
              interpreterPath
              activationMode
              system
              containerSystem
              ;
            environmentOutPath = "${containerPkgs.hello}";
            floxBin = "${containerPkgs.hello}";
          }).drvPath;
      };
    };
in
pkgs.dockerTools.streamLayeredImage buildLayeredImageArgs
