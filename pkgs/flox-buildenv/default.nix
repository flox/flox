{
  cacert,
  darwin,
  coreutils,
  flox-activation-scripts,
  flox-pkgdb,
  glibcLocalesUtf8,
  lib,
  nix,
  perl,
  runCommandNoCC,
  stdenv,
  writers,
  writeText,
}:
let
  pname = "flox-buildenv";
  version = "0.0.1";
  buildenv = (writers.writeBash "buildenv" (builtins.readFile ../../buildenv/buildenv.bash));
  buildenv_nix = ../../buildenv/buildenv.nix;
  builder_pl = ../../buildenv/builder.pl;
  activationScripts_out = flox-activation-scripts.out;
  activationScripts_build_wrapper = flox-activation-scripts.build_wrapper;
  defaultEnvrc = writeText "default.envrc" (
    ''
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
    ''
  );
in
runCommandNoCC "${pname}-${version}"
  {
    inherit
      coreutils
      nix
      pname
      version
      activationScripts_out
      activationScripts_build_wrapper
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

    cp ${builder_pl} "$out/lib/builder.pl"
    chmod +x "$out/lib/builder.pl"
    substituteAllInPlace "$out/lib/builder.pl"

    cp ${buildenv_nix} "$out/lib/buildenv.nix"
    substituteAllInPlace "$out/lib/buildenv.nix"
  ''
