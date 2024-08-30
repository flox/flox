{
  runCommandNoCC,
  bashInteractive,
  coreutils,
  gitMinimal,
  gnugrep,
  gnused,
  gnutar,
  jq,
  ld-floxlib,
  nix,
  substituteAll,
}: let
  flox-build-mk = substituteAll {
    name = "flox-build.mk";
    src = ../../package-builder/flox-build.mk;
    ld_floxlib = ld-floxlib; # Cannot inherit attributes containing "-".
    inherit bashInteractive coreutils gitMinimal gnugrep gnused gnutar jq nix;
  };
in
  runCommandNoCC "flox-package-builder" {} ''
    # include builder makefile and utility nix script
    mkdir -p $out/libexec
    cp ${flox-build-mk} $out/libexec/flox-build.mk
    cp ${../../package-builder/build-manifest.nix} $out/libexec/build-manifest.nix
  ''
