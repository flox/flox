# Capacitor API
{
  lib,
  self,
}: args @ {
  name ? "floxShell",
  # A path to a buildEnv that will be loaded by the shell.
  # We assume that the buildEnv contains an ./env.bash script.
  packages ? [],
  meta ? {},
  passthru ? {},
  env ? {},
  manifestPath ? null,
  pkgs,
  ...
}:
# TODO: let packages' = if builtins.isList builtins.isSet
let
  bashPath = "${bashInteractive}/bin/bash";
  stdenv = writeTextFile {
    name = "naked-stdenv";
    destination = "/setup";
    text = ''
      # Fix for `nix develop`
      : ''${outputs:=out}
      runHook() {
        eval "$shellHook"
        unset runHook
      }
    '';
  };
  inherit
    (pkgs)
    buildEnv
    writeTextDir
    system
    coreutils
    bashInteractive
    writeTextFile
    ;
  rest = builtins.removeAttrs args [
    "name"
    "profile"
    "packages"
    "meta"
    "passthru"
    "env"
    "manifestPath"
    "pkgs"
  ];
  envToBash = name: value: "export ${name}=${lib.escapeShellArg (toString value)}";
  envBash = writeTextDir "env.bash" ''
    export PATH="@DEVSHELL_DIR@/bin:$PATH"
    ${builtins.concatStringsSep "\n" (builtins.attrValues (builtins.mapAttrs envToBash (args.env or {})))}
    ${args.postShellHook or ""}
  '';
  profile = let
    env = derivation {
      name = "profile";
      builder = "builtin:buildenv";
      inherit system;
      manifest = "/dummy";
      derivations = map (x: ["true" (x.meta.priority or 5) 1 x]) args.packages;
    };
    manifestJSON = builtins.toJSON {
      elements =
        map (
          v:
            if v ? meta.publishData.element
            then let
              el = v.meta.publishData.element;
            in
              el
              // {
                active = true;
                attrPath = builtins.concatStringsSep "." el.attrPath;
                priority = v.meta.priority or 5;
              }
            else {
              active = true;
              storePaths = [(builtins.unsafeDiscardStringContext v)];
            }
        )
        args.packages;
      version = 2;
    };
    manifestFile = builtins.toFile "profile" manifestJSON;
    manifest = derivation {
      name = "profile";
      inherit system;
      builder = "/bin/sh";
      args = [
        "-c"
        "echo ${env}; ${coreutils}/bin/mkdir $out; ${coreutils}/bin/cp ${
          if manifestPath == null
          then manifestFile
          else manifestPath
        } $out/manifest.json"
      ];
    };

    # Last wrapper is to incorporate priorities. These can be optimized by
    # inlining, but are also accomplishing separate tasks.
    # TODO: optimize
    pre-wrapper = derivation {
      name = "wrapper";
      system = system;
      builder = "builtin:buildenv";
      manifest = "unused";
      derivations =
        map (x: ["true" (x.meta.priority or 5) 1 x]) (args.packages ++ [envBash]);
    };
  in
    # note: this allows for an input-addressed approach for an environment to self-activate
    buildEnv ({
        name = "wrapper";
        paths = [manifest pre-wrapper];

        postBuild = ''
          rm $out/manifest.nix
          rm $out/env.bash ; substitute ${envBash}/env.bash $out/env.bash --subst-var-by DEVSHELL_DIR $out
          ${args.postBuild or ""}
        '';
      }
      // (builtins.removeAttrs rest ["postShellHook" "shellHook" "preShellHook" "postBuild"]));
in
  (derivation ({
      inherit name system;
      outputs = ["out"];

      # `nix develop` actually checks and uses builder. And it must be bash.
      builder = bashPath;

      # Bring in the dependencies on `nix-build`
      args = ["-ec" "${coreutils}/bin/ln -s ${profile} $out; exit 0"];

      # $stdenv/setup is loaded by nix-shell during startup.
      # https://github.com/nixos/nix/blob/377345e26f1ac4bbc87bb21debcc52a1d03230aa/src/nix-build/nix-build.cc#L429-L432
      stdenv = stdenv;

      # The shellHook is loaded directly by `nix develop`. But nix-shell
      # requires that other trampoline.
      shellHook = ''
        # Remove all the unnecessary noise that is set by the build env
        unset NIX_BUILD_TOP NIX_BUILD_CORES NIX_STORE
        unset TEMP TEMPDIR TMP TMPDIR
        # $name variable is preserved to keep it compatible with pure shell https://github.com/sindresorhus/pure/blob/47c0c881f0e7cfdb5eaccd335f52ad17b897c060/pure.zsh#L235
        unset builder out shellHook stdenv system
        # Flakes stuff
        unset dontAddDisableDepTrack outputs
        # For `nix develop`. We get /noshell on Linux and /sbin/nologin on macOS.
        if [[ "$SHELL" == "/noshell" || "$SHELL" == "/sbin/nologin" ]]; then
          export SHELL=${bashPath}
        fi
        # Load the environment
        if [ -f "${profile}/env.bash" ]; then
          source "${profile}/env.bash"
        fi
        if [ -f "${profile}/activate" ]; then
          source "${profile}/activate"
        fi
      '';
    }
    // rest
    // (args.env or {})))
  // {inherit meta passthru;}
  // passthru
