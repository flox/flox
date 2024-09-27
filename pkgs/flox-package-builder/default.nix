{
  bashInteractive,
  coreutils,
  gitMinimal,
  gnugrep,
  gnused,
  gnutar,
  jq,
  nix,
  stdenv,
  substituteAll,
}: let
  flox-build-mk = substituteAll {
    name = "flox-build.mk";
    src = ../../package-builder/flox-build.mk;
    inherit bashInteractive coreutils gitMinimal gnugrep gnused gnutar jq nix;
  };
in
  stdenv.mkDerivation {
    pname = "package-builder";
    version = "1.0.0";
    src = builtins.path {
      name = "package-builder-src";
      path = "${./../../package-builder}";
    };
    postPatch = ''
      cp ${flox-build-mk} flox-build.mk
    '';
    # install the packages to $out/libexec/*
    makeFlags = ["PREFIX=$(out)"];
    doCheck = true;

    passthru.devShellHook = ''
      # Find the project root and set FLOX_BUILD_MK.
      if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
        REPO_ROOT="$( git rev-parse --show-toplevel; )";
        FLOX_BUILD_MK="$REPO_ROOT/package-builder/flox-build.mk";
      fi
    '';
  }
