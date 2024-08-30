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
    makeFlags = ["PREFIX=$(out)"];
    doCheck = true;
  }
