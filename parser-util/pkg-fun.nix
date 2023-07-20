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
  bats,
  gnused,
  jq,
}: let
  boost_CFLAGS = "-I" + boost + "/include";
  libExt = stdenv.hostPlatform.extensions.sharedLibrary;
  nix_INCDIR = nix.dev + "/include";
  batsWith = bats.withLibraries (p: [
    p.bats-assert
    p.bats-file
    p.bats-support
  ]);
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
    nativeBuildInputs = [
      # required for builds:
      pkg-config
      # required for tests:
      batsWith
      gnused
      jq
    ];
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
    doInstallCheck = false;
  }
# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #

