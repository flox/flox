{
  cacert,
  darwin,
  coreutils,
  flox-interpreter,
  glibcLocalesUtf8,
  lib,
  nix,
  perl,
  runCommandNoCC,
  stdenv,
  writeText,
}:
# We need to ensure that the flox-interpreter package is available.
# If it's not, we'll use the binary from the environment.
# Build or evaluate this package with `--option pure-eval false`.
assert (flox-interpreter == null) -> builtins.getEnv "FLOX_INTERPRETER" != null;
let
  pname = "flox-buildenv";
  version = "0.0.1";
  buildenv_nix = ../../buildenv/buildenv.nix;
  builder_pl = ../../buildenv/builder.pl;
  activationScripts_fallback = builtins.getEnv "FLOX_INTERPRETER";
  interpreter_out =
    if flox-interpreter != null then flox-interpreter.out else "${activationScripts_fallback}";
  interpreter_wrapper =
    if flox-interpreter != null then
      flox-interpreter.build_executable_wrapper
    else
      "${activationScripts_fallback}-build_executable_wrapper";

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
      interpreter_out
      interpreter_wrapper
      defaultEnvrc
      ;
    # Substitutions for builder.pl.
    inherit (builtins) storeDir;
    perl = perl + "/bin/perl";
  }
  ''
    mkdir -p "$out/lib"

    cp ${builder_pl} "$out/lib/builder.pl"
    chmod +x "$out/lib/builder.pl"
    substituteAllInPlace "$out/lib/builder.pl"

    cp ${buildenv_nix} "$out/lib/buildenv.nix"
    substituteAllInPlace "$out/lib/buildenv.nix"
  ''
