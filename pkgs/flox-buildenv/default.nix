{
  cacert,
  coreutils,
  darwin,
  flox-interpreter,
  flox-nix-builder,
  glibcLocalesUtf8,
  lib,
  nix,
  runCommand,
  stdenv,
  writeText,
}:
# We need to ensure that the flox-interpreter and flox-nix-builder packages are available.
# If they're not, we'll use the binaries from the environment.
# Build or evaluate this package with `--option pure-eval false`.
assert (flox-interpreter == null) -> builtins.getEnv "FLOX_INTERPRETER" != null;
assert (flox-nix-builder == null) -> builtins.getEnv "FLOX_NIX_BUILDER" != null;
let
  pname = "flox-buildenv";
  version = "0.0.1";
  buildenvLib = ../../buildenv/buildenvLib;
  buildenv_nix = ../../buildenv/buildenv.nix;
  activationScripts_fallback = builtins.getEnv "FLOX_INTERPRETER";
  nix_builder_fallback = builtins.getEnv "FLOX_NIX_BUILDER";
  interpreter_out =
    if flox-interpreter != null then flox-interpreter.out else "${activationScripts_fallback}";
  interpreter_wrapper =
    if flox-interpreter != null then
      flox-interpreter.build_executable_wrapper
    else
      "${activationScripts_fallback}-build_executable_wrapper";
  nix_builder =
    if flox-nix-builder != null then flox-nix-builder else "${nix_builder_fallback}";

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
      export PATH_LOCALE="''${PATH_LOCALE:-${darwin.locale}/share/locale}"
    ''
    + ''
      # Static environment variables
    ''
  );
in
runCommand "${pname}-${version}"
  {
    inherit
      coreutils
      nix
      pname
      version
      interpreter_out
      interpreter_wrapper
      defaultEnvrc
      nix_builder
      ;
  }
  ''
    mkdir -p "$out/bin"
    mkdir -p "$out/lib"

    cp "$nix_builder/bin/nix-builder" "$out/bin/nix-builder"
    chmod +x "$out/bin/nix-builder"

    cp ${buildenv_nix} "$out/lib/buildenv.nix"
    substituteAllInPlace "$out/lib/buildenv.nix"

    cp -r ${buildenvLib} "$out/lib/buildenvLib"
  ''
