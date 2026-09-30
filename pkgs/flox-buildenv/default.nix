{
  callPackage,
  coreutils,
  flox-activations,
  flox-interpreter,
  nix,
  runCommand,
  writeText,
}:
# We need to ensure that the flox-interpreter package is available.
# If it's not, we'll use the binary from the environment.
# Build or evaluate this package with `--option pure-eval false`.
assert (flox-interpreter == null) -> builtins.getEnv "FLOX_INTERPRETER" != null;
let
  pname = "flox-buildenv";
  version = "0.0.1";
  buildenvLib = ../../buildenv/buildenvLib;
  buildenv_nix = ../../buildenv/buildenv.nix;
  builder_pl = ../../buildenv/builder.pl;
  BuilderLibs_pm = ../../buildenv/BuilderLibs.pm;
  activationScripts_fallback = builtins.getEnv "FLOX_INTERPRETER";
  interpreter_out =
    if flox-interpreter != null then flox-interpreter.out else "${activationScripts_fallback}";
  interpreter_wrapper =
    if flox-interpreter != null then
      flox-interpreter.build_executable_wrapper
    else
      "${activationScripts_fallback}-build_executable_wrapper";
  flox_activations_out = flox-activations.out;

  # Header of the environment's activate.d/envrc; the manifest's [vars] are
  # appended to it by buildenv.nix. The environment itself sets no defaults
  # for the certificate or locale variables: activations, services and
  # containers go through flox-activations, which exports them from
  # flox-core's default_nix_env_vars, and a local `flox build`, which
  # sources this file directly, inherits them from the flox process.
  # Duplicating them here only obscured which value a user ended up with.
  defaultEnvrc = writeText "default.envrc" ''
    # Static environment variables
  '';
  perl = callPackage ./flox-perl.nix {
    # Script which determines the modules to keep.
    perlScript = BuilderLibs_pm;
  };
in
runCommand "${pname}-${version}"
  {
    inherit
      coreutils
      nix
      pname
      version
      interpreter_out
      flox_activations_out
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

    cp ${BuilderLibs_pm} "$out/lib/BuilderLibs.pm"

    cp ${buildenv_nix} "$out/lib/buildenv.nix"
    substituteAllInPlace "$out/lib/buildenv.nix"

    cp -r ${buildenvLib} "$out/lib/buildenvLib"
  ''
