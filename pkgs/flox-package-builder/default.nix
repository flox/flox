{
  bashInteractive,
  coreutils,
  daemonize,
  getopt,
  gitMinimal,
  gnugrep,
  gnused,
  gnutar,
  jq,
  nix,
  stdenv,
  t3,
}:
stdenv.mkDerivation {
  pname = "package-builder";
  version = "1.0.0";
  src = builtins.path {
    name = "package-builder-src";
    path = "${./../../package-builder}";
  };
  postPatch = ''
    # Need to perform substitutions within derivation for access to $out.
    for i in build-manifest.nix env-filter.bash flox-build.mk; do
      bashInteractive=${bashInteractive} \
      coreutils=${coreutils} \
      daemonize=${daemonize} \
      getopt=${getopt} \
      gitMinimal=${gitMinimal} \
      gnugrep=${gnugrep} \
      gnused=${gnused} \
      gnutar=${gnutar} \
      jq=${jq} \
      nix=${nix} \
      t3=${t3} substituteAllInPlace $i
    done
  '';
  # install the packages to $out/libexec/*
  makeFlags = [ "PREFIX=$(out)" ];
  doCheck = true;
}
