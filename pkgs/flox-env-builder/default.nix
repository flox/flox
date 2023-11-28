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
  runCommand,
  # sql-builder,
}: let
  profile_d_scripts = runCommand "profile-d-scripts" {} ''
    mkdir -p $out/etc/profile.d
    cp -r ${../../assets/mkEnv/profile.d}/* $out/etc/profile.d/
  '';

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
    boost_CPPFLAGS = "-I" + boost.dev.outPath + "/include";
    pkgdb_CLFAGS = "-I" + flox-pkgdb.outPath + "/include";

    PROFILE_D_SCRIPT_DIR = profile_d_scripts;
    SET_PROMPT_BASH_SH = "${../../assets/mkEnv/set-prompt-bash.sh}";

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

