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
    pname = "flox-cpp";
    version = builtins.replaceStrings ["\n"] [""] (builtins.readFile "${src}/version");

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
    nix_INCDIR = nix.dev.outPath + "/include";
    boost_CFLAGS = "-I" + boost.outPath + "/include";
    pkgdb_CLFAGS = "-I" + flox-pkgdb.outPath + "/include";

    ACTIVATION_SCRIPT_BIN = activationScript;

    libExt = stdenv.hostPlatform.extensions.sharedLibrary;
    # sql_builder_CFLAGS = "-I" + sql-builder.outPath + "/include";
    # SEMVER_PATH = semver.outPath + "/bin/semver";
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

