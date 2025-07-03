{
  # nixpkgs providing `lib` and `stdenv` (`runCommand`)
  # this is overridden to point to the nixpkgs used to build flox by the caller
  nixpkgs-url ? "github:flox/nixpkgs/stable",
  pkgs ? (builtins.getFlake nixpkgs-url).legacyPackages.${builtins.currentSystem},
  pname,
  version,
  flox-env, # environment from which package is built
  build-wrapper-env, # environment with which to wrap contents of bin, sbin
  install-prefix ? null, # optional
  srcTarball ? null, # optional
  buildDeps ? [ ], # optional
  buildScript ? null, # optional
  buildCache ? null, # optional
}:
# First a few assertions to ensure that the inputs are consistent.
# buildCache is only meaningful with a build script
assert (buildCache != null) -> (buildScript != null);
# srcTarball is only required with a build script
assert (srcTarball != null) -> (buildScript != null);
let
  flox-env-package = builtins.storePath flox-env;
  build-wrapper-env-package = builtins.storePath build-wrapper-env;

  # After a successful build we want to replace references to `flox-env-package`
  # with references to `build-wrapper-env-package`
  # (a reduced subset of the complete develop environment)
  # in order to reduce the minimal closure of the result.
  # Replacing paths, particularly in binaries,
  # requires that both paths have equal lengths.
  # Hence we create a (shallow) copy of `flox-env-package`
  # named `developcopy-build-${pname}`.
  #
  # Note that we expect the name of `flox-env-package` to follow the pattern
  # "environment-build-${pname}" and that
  # len(developcopy-build-) ==
  # len(environment-build-)
  #
  # In the name of expediency we opt for higher coupling here,
  # but should eventually land on a more robust alternative using padding.
  #
  # Note further, that because `builtins.path` does not copy references,
  # we _also_ add `flox-env-package` to the closure below.
  #
  develop-copy-env-package = builtins.path {
    path = flox-env-package;
    name = "developcopy-build-${pname}";
  };
  buildInputs = [ build-wrapper-env-package ];
  buildDepInputs = map (d: builtins.storePath d) buildDeps;
  install-prefix-contents = /. + install-prefix;
  buildScript-contents = /. + buildScript;
  buildCache-tar-contents = if (buildCache == null) then null else (/. + buildCache);

  dollar_out_bin_copy_hints = ''
    echo "  - copy a single file with 'mkdir -p \$out/bin && cp file \$out/bin'" 1>&2
    echo "  - copy a bin directory with 'mkdir \$out && cp -r bin \$out'" 1>&2
    echo "  - copy multiple files with 'mkdir -p \$out/bin && cp bin/* \$out/bin'" 1>&2
    echo "  - copy files from an Autotools project with 'make install PREFIX=\$out'" 1>&2
  '';
  dollar_out_error = ''
    echo "❌  ERROR: Build command did not copy outputs to '\$out'." 1>&2
    ${dollar_out_bin_copy_hints}
  '';
  dollar_out_no_bin_warning = ''
    echo "⚠️  WARNING: No executables found in '\$out/bin'." 1>&2
    echo "Only executables in '\$out/bin' will be available on the PATH." 1>&2
    echo "If your build produces executables, make sure they are copied to '\$out/bin'." 1>&2
    ${dollar_out_bin_copy_hints}
  '';
  name = "${pname}-${version}";

  # Custom version of makeWrapper that overrides the choice of shell
  # to be the same as that used for the build wrapper environment.
  makeBuildWrapper = pkgs.makeWrapper.overrideAttrs (oldAttrs: {
    shell = build-wrapper-env-package + "/libexec/build-wrapper-bash";
  });
in
pkgs.runCommandNoCC name
  {
    inherit
      buildInputs # required to include build wrapper environment in NIX_LDFLAGS
      flox-env-package # required to bring in all dependencies
      develop-copy-env-package # shallow copy with no dependencies
      srcTarball
      buildDepInputs
      pname
      version
      ;
    nativeBuildInputs =
      with pkgs;
      [
        findutils
        gnutar
        gnused
        t3
      ]
      ++ [ makeBuildWrapper ] # provides makeShellWrapper()
      ++ pkgs.stdenv.defaultNativeBuildInputs
      ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [ darwin.autoSignDarwinBinariesHook ];
    outputs =
      [
        "out"
      ]
      ++ pkgs.lib.optionals (buildScript != null) [
        "log"
      ]
      ++ pkgs.lib.optionals (buildCache != null) [ "buildCache" ];
    # We previously used `disallowedReferences` to prevent builds from referencing
    # the "develop" environment, but that was too strict and caused issues with
    # the "log" and "buildCache" outputs in particular. Now we instead inspect the
    # closure once the build is complete to ensure that it only contains packages
    # found only in the build wrapper closure.
    # disallowedReferences = [ flox-env-package ];
  }
  (
    (
      # Assume this script was called after an impure/non-sandboxed build.
      if (buildScript == null) then
        # local mode
        if !builtins.pathExists install-prefix then
          ''
            ${dollar_out_error}
            exit 1
          ''
        else
          ''
            # If no build script is provided copy the contents of install prefix
            # to the output directory, rewriting path references as we go.
            if [ -d ${install-prefix-contents} ]; then
              mkdir $out
              tar -C ${install-prefix-contents} -c --mode=u+w -f - . | \
                sed --binary "s%${install-prefix}%$out%g" | \
                tar -C $out -xf -
            else
              cp ${install-prefix-contents} $out
              sed --binary "s%${install-prefix}%$out%g" $out
            fi
          ''
      # Assume we perform a full sandboxed build.
      else
        # sandbox mode
        ''
          # Print the checksums of the inputs to the build script.
          echo "---"
          echo "Input checksums:"
          md5sum \
            ${/. + srcTarball} \
            ${buildScript-contents} \
            ${pkgs.lib.optionalString (buildCache-tar-contents != null) buildCache-tar-contents}
          echo "---"
          # If the build script is provided, then it's expected that we will
          # invoke it from within the sandbox to create $out. The choice of
          # pure or impure mode occurs outside of this script as the derivation
          # is instantiated.
          source $stdenv/setup # is this necessary?

          # Set HOME to a _writable_ directory in the build sandbox.
          # <https://github.com/flox/flox/issues/2092>
          export HOME="$PWD"

          # We are currently in /build, and TMPDIR is also set to /build, so
          # we need to extract the source and work in a subdirectory to avoid
          # populating our build cache with a bunch of temporary files.
          mkdir $name && cd $name

          # We pass and extract the source as a tarball to preserve timestamps.
          # Passing the source as a directory would cause the timestamps to be
          # set to the UNIX epoch as happens with all files in the Nix store,
          # which would be older than the intermediate compilation artifacts.
          tar -xpf ${/. + srcTarball}

          # Extract contents of the cache, if it exists.
          ${
            if buildCache-tar-contents == null then
              ":"
            else
              ''
                tar --skip-old-files -xpf ${buildCache-tar-contents}
              ''
          }

          # Perform the build using a _copy_ of the "develop" environment
          # found in a storepath with the same strlen as the "build-wrapper"
          # environment, so that following a successful build we can replace
          # references to the former with the latter as we copy the output
          # to its final path.

          ${
            if buildCache == null then
              ''
                # When not preserving a cache we just run the build normally.

                # flox-activations needs runtime dir for activation state dir
                # TMP will be set to something like
                # /private/tmp/nix-build-file-0.0.0.drv-0
                # N.B. not using t3 --forcecolor option because Nix sandbox
                # strips color codes from output anyway.
                FLOX_SRC_DIR=$(pwd) FLOX_RUNTIME_DIR="$TMP" \
                  ${develop-copy-env-package}/activate --env ${develop-copy-env-package} \
                    --mode build --env-project $(pwd) -- \
                      t3 --relative $log -- bash -e ${buildScript-contents}
              ''
            else
              ''
                # If the build fails we still want to preserve the build cache, so we
                # remove $out on failure and allow the Nix build to proceed to write
                # the result symlink.

                # flox-activations needs runtime dir for activation state dir
                # TMP will be set to something like
                # /private/tmp/nix-build-file-0.0.0.drv-0

                # See flox-build.mk for a detailed explanation of why we use a nested
                # activation when performing builds.
                FLOX_SRC_DIR=$(pwd) FLOX_RUNTIME_DIR="$TMP" \
                  ${develop-copy-env-package}/activate --env ${develop-copy-env-package} \
                    --mode build --env-project $(pwd) -- \
                      t3 --relative $log -- bash -e ${buildScript-contents} || \
                ( rm -rf $out && echo "flox build failed (caching build dir)" | tee $out 1>&2 )
              ''
          }

          # Rewrite references to temporary build wrapper in "out".
          if [ -e "$out" ]; then
            bn="$(basename $out)"
            mv "$out" "$TMPDIR/$bn"
            if [ -d "$TMPDIR/$bn" ]; then
              mkdir "$out"
              tar -C "$TMPDIR/$bn" -c --mode=u+w -f - . | \
                sed --binary "s%${develop-copy-env-package}%${build-wrapper-env-package}%g" | \
                tar -C "$out" -xf -
            else
              sed --binary "s%${develop-copy-env-package}%${build-wrapper-env-package}%g" < "$TMPDIR/$bn" > "$out"
            fi
            rm -rf "$TMPDIR/$bn"
          fi
        ''
    )
    + ''
      # Check that the build populated $out.
      if [ ! -e "$out" ]; then
        ${dollar_out_error}
        exit 1
      fi

      # Take inventory of executables found in bin, sbin, and libexec.
      declare -a bin sbin libexec;
      shopt -s nullglob
      for i in bin sbin libexec; do
        for j in $out/$i/*; do
          relpath="''${j#"$out/"}"
          if [ ! -f "$j" ]; then
            # Don't warn about non-files in libexec.
            if [ "$i" != "libexec" ]; then
              echo "⚠️  WARNING: \$out/$relpath is not a file." 1>&2
            fi
          elif [ ! -x "$j" ]; then
            # Don't warn about non-executable files in libexec.
            if [ "$i" != "libexec" ]; then
              echo "⚠️  WARNING: \$out/$relpath is not executable." 1>&2
            fi
          else
            eval "$i+=($j)"
          fi
        done
      done

      # Check if there are binaries in $out/bin
      if [ ''${#bin[@]} -eq 0 ]; then
        ${dollar_out_no_bin_warning}
      fi

      # Warn about executables in $out not found in $out/{bin,sbin,libexec}.
      # Also don't warn about shared libraries that compilers mark executable
      # by default, however unwise that may be. See:
      # https://www.technovelty.org/linux/shared-libraries-and-execute-permissions.html
      declare -a stray_binaries
      stray_binaries=($(
        find "$out" -type f -executable \
          -not -name "*.so" \
          -not -name "*.dylib" \
          -not -path "$out/bin/*" \
          -not -path "$out/sbin/*" \
          -not -path "$out/libexec/*" \
          -printf "%P\n"
        for i in bin sbin libexec; do
          for j in $out/$i/*; do
            if [ -d "$j" ]; then
              find "$out" -type f -executable \
                -path "$j/*" \
                -not -name "*.so" \
                -not -name "*.dylib" \
                -printf "%P\n"
            fi
          done
        done
      ))
      if [ ''${#stray_binaries[@]} -gt 0 ]; then
        # [sic] ignored in 'nix build -L' output:
        # <https://github.com/NixOS/nix/issues/11991>
        echo "" 1>&2
        echo "HINT: The following executables were found outside of '\$out/bin':" 1>&2
        for binary in ''${stray_binaries[@]}; do
          echo "  - $binary" 1>&2
        done
      fi

      for prog in ''${bin[@]} ''${sbin[@]} ''${libexec[@]}; do
        # Start by patching shebangs in executables, making sure to prefer the
        # build wrapper environment over the "develop" environment. Note that
        # the paths substituted come from executables found in `$PATH` at the
        # time that this function is invoked, so we automatically patch with
        # paths found in $FLOX_ENV.
        patchShebangs "$prog"

        # Wrap contents of executables with ${build-wrapper-env-package}/wrapper
        if [ -L "$prog" ]; then
          : # You cannot wrap a symlink, so just leave it be?
        else
          assertExecutable "$prog"
          hidden="$(dirname "$prog")/.$(basename "$prog")"-wrapped
          mv "$prog" "$hidden"
          # TODO: we shouldn't need to set FLOX_RUNTIME_DIR here
          makeShellWrapper "${build-wrapper-env-package}/wrapper" "$prog" \
            --inherit-argv0 \
            --set FLOX_MANIFEST_BUILD_OUT "$out" \
            --set FLOX_RUNTIME_DIR "/tmp" \
            --run 'export FLOX_SET_ARG0="$0"' \
            --add-flags "--env ${build-wrapper-env-package}" \
            --add-flags -- \
            --add-flags "$hidden"
        fi
      done
    ''
    + pkgs.lib.optionalString (buildCache != null) ''
      # Only tar the files to avoid differences in directory {a,c,m}times.
      # Sort the files to keep the output stable across builds.
      # Avoid compressing with gzip because that is not stable across
      # invocations on Mac only. Experimentation shows that xz and bzip2
      # compression is stable on both Mac and Linux, but that can be slow,
      # and we probably don't actually need to compress the build cache
      # because we actively delete the old copy as we create a new one.
      find . -type f | sort | tar -c --no-recursion -f $buildCache -T -
    ''
    + pkgs.lib.optionalString pkgs.stdenv.isDarwin ''
      signDarwinBinariesIn "$out"
    ''
  )
