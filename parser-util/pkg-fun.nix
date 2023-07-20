# ============================================================================ #
#
#
#
# ---------------------------------------------------------------------------- #
{
  stdenv,
  pkg-config,
  nlohmann_json,
  nix,
  boost,
}: let
  boost_CFLAGS = "-I" + boost + "/include";
  libExt = stdenv.hostPlatform.extensions.sharedLibrary;
  nix_INCDIR = nix.dev + "/include";
in
  stdenv.mkDerivation {
    pname = "parser-util";
    version = "0.1.0";
    src = builtins.path {
      path = ./.;
      filter = name: type: let
        bname = baseNameOf name;
        ignores = ["pkg-fun.nix" ".gitignore" "out" "bin"];
        notIgnored = ! (builtins.elem bname ignores);
        notObject = (builtins.match ".*\\.o" name) == null;
        notResult = (builtins.match "result(-*)?" bname) == null;
        notJSON = (builtins.match ".*\\.json" name) == null;
      in
        notIgnored && notObject && notResult && notJSON;
    };
    inherit boost_CFLAGS nix_INCDIR libExt;
    nativeBuildInputs = [pkg-config];
    buildInputs = [nlohmann_json nix.dev boost];
    makeFlags = [
      "libExt='${libExt}'"
      "boost_CFLAGS='${boost_CFLAGS}'"
      "nix_INCDIR='${nix_INCDIR}'"
    ];
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

