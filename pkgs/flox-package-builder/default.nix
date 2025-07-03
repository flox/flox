{
  bashInteractive,
  coreutils,
  daemonize,
  findutils,
  getopt,
  gitMinimal,
  gnugrep,
  gnused,
  gnutar,
  jq,
  nix,
  shellcheck,
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
  nativeBuildInputs = [ shellcheck ];
  postPatch = ''
    # Need to perform substitutions within derivation for access to $out.
    for i in build-manifest.nix flox-build.mk validate-build.bash; do
      bashInteractive=${bashInteractive} \
      coreutils=${coreutils} \
      daemonize=${daemonize} \
      findutils=${findutils} \
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
