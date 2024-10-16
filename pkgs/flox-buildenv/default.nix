{
  bash,
  cacert,
  darwin,
  coreutils,
  findutils,
  flox-activation-scripts,
  flox-pkgdb,
  getopt,
  glibcLocalesUtf8,
  jq,
  lib,
  nix,
  nixpkgsClone,
  perl,
  runCommandNoCC,
  stdenv,
  writers,
  writeText,
}: let
  pname = "flox-buildenv";
  version = "0.0.1";
  nixpkgsBuildenvRoot = nixpkgsClone + "/pkgs/build-support/buildenv";
  buildenv = (
    writers.writeBash "buildenv" (
      builtins.readFile ./buildenv.bash
    )
  );
  buildenv_nix = ./buildenv.nix;
  pkgdb = (
    writers.writeBash "pkgdb" (
      builtins.readFile ./pkgdb.bash
    )
  );
  builder_pl = ./builder.pl;
  builder_pl_patch = ./builder.pl.patch;
  build_packages_jq = ./build-packages.jq;
  activationScripts = flox-activation-scripts;
  defaultEnvrc = writeText "default.envrc" (''
      # Default environment variables
      export SSL_CERT_FILE="''${SSL_CERT_FILE:-${cacert}/etc/ssl/certs/ca-bundle.crt}"
      export NIX_SSL_CERT_FILE="''${NIX_SSL_CERT_FILE:-''${SSL_CERT_FILE}}"
    ''
    + lib.optionalString stdenv.isLinux ''
      export LOCALE_ARCHIVE="''${LOCALE_ARCHIVE:-${glibcLocalesUtf8}/lib/locale/locale-archive}"
    ''
    + lib.optionalString stdenv.isDarwin ''
      export NIX_COREFOUNDATION_RPATH="''${NIX_COREFOUNDATION_RPATH:-"${darwin.CF}/Library/Frameworks"}"
      export PATH_LOCALE="''${PATH_LOCALE:-${darwin.locale}/share/locale}"
    ''
    + ''
      # Static environment variables
    '');
in
  runCommandNoCC
  "${pname}-${version}"
  {
    inherit
      coreutils
      findutils
      getopt
      jq
      nix
      pname
      version
      activationScripts
      defaultEnvrc
      ;
    # Substitutions for builder.pl.
    inherit (builtins) storeDir;
    perl = perl + "/bin/perl";
    floxPkgdb = flox-pkgdb;
  }
  ''
    mkdir -p "$out/bin" "$out/lib"
    cp ${buildenv} "$out/bin/buildenv"
    substituteAllInPlace "$out/bin/buildenv"
    cp ${pkgdb} "$out/bin/pkgdb"
    substituteAllInPlace "$out/bin/pkgdb"

    # Uncomment these lines to generate builder.pl from the Nixpkgs source.
    # cp --no-preserve=mode ${nixpkgsBuildenvRoot}/builder.pl "$out/lib/builder.pl"
    # (cd $out/lib && exec patch -p2 < ${builder_pl_patch})
    #
    # ... but in the meantime, we use a modified version of builder.pl to
    # make it easier to hack on.
    cp ${builder_pl} "$out/lib/builder.pl"
    chmod +x "$out/lib/builder.pl"
    substituteAllInPlace "$out/lib/builder.pl"

    cp ${buildenv_nix} "$out/lib/buildenv.nix"
    substituteAllInPlace "$out/lib/buildenv.nix"

    cp ${build_packages_jq} "$out/lib/build-packages.jq"
  ''
