{
  bash,
  coreutils,
  daemonize,
  fd,
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
let
  activation-scripts = substituteAllFiles {
    src = ../../assets/activation-scripts;
    files = [ "." ]; # Perform recursive substitution on all files.
    # Substitute all of the following variables.
    inherit
      bash
      coreutils
      daemonize
      fd
      findutils
      getopt
      gnused
      jq
      nawk
      ;
    # Note that substitution doesn't work with variables containing "-"
    # so we need to create and use alternative names.
    process_compose = process-compose;
    flox_activations = flox-activations;
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
    chmod +x $out/activate
    chmod -R +w $out
    patchShebangs $out/activate
    substituteInPlace $out/activate --replace-fail "__OUT__" "$out"

    # Next create the (lesser) "build_wrapper" output.
    cp -R ${activation-scripts} $build_wrapper
    chmod +x $build_wrapper/activate
    chmod -R +w $build_wrapper
    patchShebangs $build_wrapper/activate
    substituteInPlace $build_wrapper/activate --replace-fail "__OUT__" "$build_wrapper"

    # TODO: come up with neater way to master activate script for build_wrapper case.

    # Remove start-services.bash.
    rm $build_wrapper/activate.d/start-services.bash
    sed -i 's/source ".*start-services.bash"/: no services in build_wrapper script/' $build_wrapper/activate

    # Remove references to flox-activations.
    rm $build_wrapper/activate.d/attach-*.bash
    sed -i 's/source ".*attach-.*.bash"/: no attaching in build_wrapper script/' $build_wrapper/activate
    sed -i 's/_flox_activations=.*/_flox_activations=true/' \
        $build_wrapper/activate $build_wrapper/activate.d/start.bash

    # That's the build done, now shellcheck the results.
    ${shellcheck}/bin/shellcheck --external-sources --check-sourced \
      $out/activate \
      $out/activate.d/bash \
      $out/activate.d/set-prompt.bash \
      $out/etc/profile.d/* \
      $build_wrapper/activate \
      $build_wrapper/activate.d/bash \
      $build_wrapper/activate.d/set-prompt.bash \
      $build_wrapper/etc/profile.d/*

    # Finally check the formatting of the scripts with shfmt. Only check
    # $out as the contents of $build_wrapper will be virtually identical.
    cp ${editorconfig} $out/.editorconfig
    # This will only catch extensions and shebangs that `shfmt --find` knows about.
    ${shfmt}/bin/shfmt --diff $out
    rm $out/.editorconfig
  ''
