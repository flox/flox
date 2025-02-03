{
  bashInteractive,
  coreutils,
  daemonize,
  gitMinimal,
  gnugrep,
  gnused,
  gnutar,
  jq,
  nix,
  stdenv,
  substituteAll,
  t3,
}:
let
  build-manifest-nix = substituteAll {
    name = "build-manifest.nix";
    src = ../../package-builder/build-manifest.nix;
    inherit t3;
  };
  flox-build-mk = substituteAll {
    name = "flox-build.mk";
    src = ../../package-builder/flox-build.mk;
    inherit
      bashInteractive
      coreutils
      daemonize
      gitMinimal
      gnugrep
      gnused
      gnutar
      jq
      nix
      t3
      ;
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
    cp ${build-manifest-nix} build-manifest.nix
  '';
  # install the packages to $out/libexec/*
  makeFlags = [ "PREFIX=$(out)" ];
  doCheck = true;
}
