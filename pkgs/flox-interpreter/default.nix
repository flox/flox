{
  bash,
  coreutils,
  daemonize,
  findutils,
  flox-activations,
  getopt,
  gnused,
  iconv,
  jq,
  ld-floxlib,
  nawk,
  process-compose,
  runCommandNoCC,
  shellcheck,
  shfmt,
  stdenv,
  substituteAllFiles,
  util-linux,
}:
# We need to ensure that the flox-activations package is available.
# If it's not, we'll use the binary from the environment.
# Build or evaluate this package with `--option pure-eval false`.
assert (flox-activations == null) -> builtins.getEnv "FLOX_ACTIVATIONS_BIN" != null;
let
  environment-interpreter-with-paths = substituteAllFiles {
    src = ../../assets/environment-interpreter;
    files = [ "." ]; # Perform recursive substitution on all files.
    # Substitute all of the following variables.
    inherit
      bash
      coreutils
      daemonize
      findutils
      getopt
      gnused
      jq
      nawk
      ;
    # Note that substitution doesn't work with variables containing "-"
    # so we need to create and use alternative names.
    process_compose = process-compose;
    # If the flox-activations package is available, use it,
    # otherwise copy the binary from the environment into the store,
    # so that sandboxed builds and flox built containers can access it.
    flox_activations =
      if flox-activations != null then
        "${flox-activations}/bin/flox-activations"
      else
        "${builtins.path { path = builtins.getEnv "FLOX_ACTIVATIONS_BIN"; }}";
    util_linux = util-linux;
    # Make clear when packages are not available on Darwin.
    ld_floxlib = if stdenv.isLinux then ld-floxlib else "__LINUX_ONLY__";
    iconv = if stdenv.isLinux then iconv else "__LINUX_ONLY__";
  };

  # Create editorconfig for use in `shfmt` check. Note that some versions
  # of Nix don't support `.` in name.
  editorconfig = builtins.path {
    name = "editorconfig";
    path = ../../.editorconfig;
  };

in
runCommandNoCC "flox-interpreter"
  {
    nativeBuildInputs = [ gnused ];
    outputs = [
      "out"
      "build_executable_wrapper"
    ];
  }
  ''
    # Create the "out" output.
    mkdir -p $out
    cp -R ${environment-interpreter-with-paths}/common/* $out --no-preserve=mode
    cp -R ${environment-interpreter-with-paths}/activate/* $out --no-preserve=mode
    chmod -R +w $out

    chmod +x $out/activate
    patchShebangs $out/activate

    mv $out/activate.d/trace.bash $out/activate.d/trace
    chmod +x $out/activate.d/trace
    patchShebangs $out/activate.d/trace

    # Replace __OUT__ with the output path for both outputs.
    substituteInPlace $out/activate --replace-fail "__OUT__" "$out"

    # That's the build done, now shellcheck the results.
    ${shellcheck}/bin/shellcheck --external-sources --check-sourced \
      $out/activate \
      $out/activate.d/generate-bash-startup-commands.bash \
      $out/activate.d/generate-fish-startup-commands.bash \
      $out/activate.d/generate-tcsh-startup-commands.bash \
      $out/activate.d/set-prompt.bash \
      $out/activate.d/source-profile-d.bash \
      $out/etc/profile.d/*

    # Finally check the formatting of the scripts with shfmt.
    cp ${editorconfig} $out/.editorconfig
    # This will only catch extensions and shebangs that `shfmt --find` knows about.
    ${shfmt}/bin/shfmt --diff $out
    rm $out/.editorconfig

    # Next create the (lesser) "wrapper" output.

    mkdir -p $build_executable_wrapper
    chmod +w $out
    cp -R ${environment-interpreter-with-paths}/common/* $build_executable_wrapper --no-preserve=mode
    cp -R ${environment-interpreter-with-paths}/wrapper/* $build_executable_wrapper --no-preserve=mode
    chmod -R +w $build_executable_wrapper

    # make the wrapper and trace script executable
    chmod +x $build_executable_wrapper/wrapper
    patchShebangs $build_executable_wrapper/wrapper

    mv $build_executable_wrapper/activate.d/trace.bash $build_executable_wrapper/activate.d/trace
    chmod +x $build_executable_wrapper/activate.d/trace
    patchShebangs $build_executable_wrapper/activate.d/trace

    # Replace __OUT__ with the output path for both outputs.
    substituteInPlace $build_executable_wrapper/wrapper --replace-fail "__OUT__" "$build_executable_wrapper"

    ${shellcheck}/bin/shellcheck --external-sources --check-sourced \
      $build_executable_wrapper/wrapper \
      $build_executable_wrapper/activate.d/* \
      $build_executable_wrapper/etc/profile.d/*

    # Finally check the formatting of the scripts with shfmt.
    cp ${editorconfig} $build_executable_wrapper/.editorconfig
    # This will only catch extensions and shebangs that `shfmt --find` knows about.
    ${shfmt}/bin/shfmt --diff $build_executable_wrapper
    rm $build_executable_wrapper/.editorconfig
  ''
