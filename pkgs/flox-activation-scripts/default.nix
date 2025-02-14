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
  activation-scripts = substituteAllFiles {
    src = ../../assets/activation-scripts;
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
runCommandNoCC "flox-activation-scripts"
  {
    nativeBuildInputs = [ gnused ];
    outputs = [
      "out"
      "build_wrapper"
    ];
  }
  ''
    # Create the "out" output.
    cp -R ${activation-scripts} $out
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

    # Finally check the formatting of the scripts with shfmt. Only check
    # $out as the contents of $build_wrapper will be virtually identical.
    cp ${editorconfig} $out/.editorconfig
    # This will only catch extensions and shebangs that `shfmt --find` knows about.
    ${shfmt}/bin/shfmt --diff $out
    rm $out/.editorconfig

    # Remove the wrapper script from the output,
    # we dont need that in environment interpreters.
    rm $out/wrapper

    # Next create the (lesser) "build_wrapper" output.
    # TODO: come up with neater way to master activate script for build_wrapper case.

    mkdir -p $build_wrapper
    cp ${activation-scripts}/wrapper $build_wrapper/

    # create activate.d directory and copy the required scripts
    mkdir -p $build_wrapper/activate.d
    cp ${activation-scripts}/activate.d/source-profile-d.bash $build_wrapper/activate.d/
    cp ${activation-scripts}/activate.d/trace.bash $build_wrapper/activate.d/

    # copy the etc/profile.d directory
    cp -R ${activation-scripts}/etc $build_wrapper/

    # make the wrapper and trace script executable
    chmod +x $build_wrapper/wrapper
    patchShebangs $build_wrapper/wrapper

    mv $build_wrapper/activate.d/trace.bash $build_wrapper/activate.d/trace
    chmod +x $build_wrapper/activate.d/trace
    patchShebangs $build_wrapper/activate.d/trace

    # Replace __OUT__ with the output path for both outputs.
    substituteInPlace $build_wrapper/wrapper --replace-fail "__OUT__" "$build_wrapper"

    ${shellcheck}/bin/shellcheck --external-sources --check-sourced \
      $build_wrapper/wrapper \
      $build_wrapper/activate.d/* \
      $build_wrapper/etc/profile.d/*
  ''
