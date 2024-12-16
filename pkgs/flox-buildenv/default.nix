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
# We need to ensure that the flox-activation-scripts package is available.
# If it's not, we'll use the binary from the environment.
# Build or evaluate this package with `--option pure-eval false`.
assert (flox-activation-scripts == null) -> builtins.getEnv "FLOX_INTERPRETER" != null;
let
  pname = "flox-buildenv";
  version = "0.0.1";
  buildenv = (writers.writeBash "buildenv" (builtins.readFile ../../buildenv/buildenv.bash));
  buildenv_nix = ../../buildenv/buildenv.nix;
  builder_pl = ../../buildenv/builder.pl;
  activationScripts_fallback = builtins.getEnv "FLOX_INTERPRETER";
  activationScripts_out =
    if flox-activation-scripts != null then
      flox-activation-scripts.out
    else
      "${activationScripts_fallback}";
  activationScripts_build_wrapper =
    if flox-activation-scripts != null then
      flox-activation-scripts.build_wrapper
    else
      "${activationScripts_fallback}-build_wrapper";

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
    pkgdb = if flox-pkgdb != null then "${flox-pkgdb}/bin/pkgdb" else "$PKGDB_BIN";
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
