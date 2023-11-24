# ============================================================================ #
#
#
#
# ---------------------------------------------------------------------------- #
{
  self,
  stdenv,
  sqlite,
  pkg-config,
  nlohmann_json,
  nix,
  boost,
  argparse,
  semver,
  flox-pkgdb,
  sqlite3pp,
  writeTextFile,
  lib,
  # sql-builder,
}: let
  activationScript = writeTextFile {
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

      . ${../../assets/mkEnv/set-prompt.sh}
      . ${../../assets/mkEnv/source-profiles.sh}
      . ${../../assets/mkEnv/run-activation-hook.sh}
    '';
  };

  src = builtins.path {
    path = "${self}/env-builder";
    filter = name: type: let
      bname = baseNameOf name;
      # Tests require internet so there's no point in including them
      ignores = ["out" "bin" "lib" "tests"];
      ext = let
        m = builtins.match ".*\\.([^.]+)" name;
      in
        if m == null
        then ""
        else builtins.head m;
      ignoredExts = ["o" "so" "dylib" "nix"];
      notIgnored =
        (! (builtins.elem bname ignores))
        && (! (builtins.elem ext ignoredExts));
      notResult = (builtins.match "result(-*)?" bname) == null;
    in
      notIgnored && notResult;
  };
in
  stdenv.mkDerivation {
    pname = "flox-env-builder";
    version = builtins.replaceStrings ["\n"] [""] (builtins.readFile "${src}/version");
    src = src;

    propagatedBuildInputs = [semver nix.dev boost];
    nativeBuildInputs = [pkg-config];
    buildInputs = [
      sqlite.dev
      nlohmann_json
      argparse
      flox-pkgdb
      sqlite3pp
      # sql-builder
    ];
    # nix_INCDIR = nix.dev.outPath + "/include";
    boost_CFLAGS = "-I" + boost.outPath + "/include";
    pkgdb_CLFAGS = "-I" + flox-pkgdb.outPath + "/include";

    ACTIVATION_SCRIPT_BIN = activationScript;
    PROFILE_D_SCRIPT_DIR = builtins.path {
      name = "profile-d-scripts";
      path = "${self}/assets/mkEnv";
      filter = path: type: let
        relativePath = lib.removePrefix "${self}/assets/mkEnv/" path;
      in
        builtins.head (lib.splitString "/" relativePath) == "profile.d";
    };

    libExt = stdenv.hostPlatform.extensions.sharedLibrary;

    configurePhase = ''
      runHook preConfigure;
      export PREFIX="$out";
      if [[ "''${enableParallelBuilding:-1}" = 1 ]]; then
        makeFlagsArray+=( '-j4' );
      fi
      runHook postConfigure;
    '';
    # Checks require internet
    doCheck = false;
    doInstallCheck = false;
  }
# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #

