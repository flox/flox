{runCommandNoCC}:
runCommandNoCC "flox-package-builder" {} ''

  # include builder makefile and unitility nix script
  mkdir -p $out/libexec

  # todo: substitute external executables and remove __FLOX_CLI_OUTPATH__
  cp ${../../package-builder/flox-build.mk} $out/libexec/flox-build.mk
  substituteInPlace $out/libexec/flox-build.mk \
    --replace "__FLOX_CLI_OUTPATH__" "$out"

  # todo: substitute external executables and remove __FLOX_CLI_OUTPATH__
  cp ${../../package-builder/build-manifest.nix} $out/libexec/build-manifest.nix
  substituteInPlace $out/libexec/build-manifest.nix \
    --replace "__FLOX_CLI_OUTPATH__" "$out"

  # todo: add links for sandbox
''
