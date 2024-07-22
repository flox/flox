{
  pkgs ? import <nixpkgs> {},
  name,
  flox-env,
  install-prefix,
  srcTarball ? null, # optional
  buildDeps ? [], # optional
  buildScript ? null, # optional
  buildCache ? null, # optional
  virtualSandbox ? "off", # optional
}:

# First a few assertions to ensure that the inputs are consistent.

# buildCache is only meaningful with a build script
assert (buildCache != null) -> (buildScript != null);
# srcTarball is only required with a build script
assert (srcTarball != null) -> (buildScript != null);

let

  flox-env-package = builtins.storePath flox-env;
  buildInputs = (
    map (d: builtins.storePath d) buildDeps
  ) ++ [flox-env-package];
  install-prefix-contents = /. + install-prefix;
  buildScript-contents = /. + buildScript;
  buildCache-tar-contents = if (buildCache == null) then null else (/. + buildCache);

in
  pkgs.runCommand name {
    inherit buildInputs srcTarball;
    nativeBuildInputs = with pkgs; [findutils gnutar gnused makeWrapper]
      ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [ darwin.autoSignDarwinBinariesHook ];
    outputs = [ "out" ] ++ pkgs.lib.optionals ( buildCache != null ) [ "buildCache" ];
  } ( (
      if (buildScript == null)
      then ''
        # If no build script is provided copy the contents of install prefix
        # to the output directory, rewriting path references as we go.
        if [ -e ${install-prefix-contents} ]; then
          if [ -d ${install-prefix-contents} ]; then
            mkdir $out
            tar -C ${install-prefix-contents} -c --mode=u+w -f - . | \
              sed --binary "s%${install-prefix}%$out%g" | \
              tar -C $out -xf -
          else
            cp ${install-prefix-contents} $out
            sed --binary "s%${install-prefix}%$out%g" $out
          fi
        else
          echo "ERROR: build did not produce expected \$out (${install-prefix})" 1>&2
          exit 1
        fi
        ${pkgs.lib.optionalString pkgs.stdenv.isDarwin ''
          signDarwinBinariesInAllOutputs
        ''}
      ''
      else ''
        echo "---"
        echo "Input checksums:"
        md5sum \
          ${/. + srcTarball} \
          ${buildScript-contents} \
          ${pkgs.lib.optionalString (
            buildCache-tar-contents != null
          ) buildCache-tar-contents}
        echo "---"
        # If the build script is provided, then it's expected that we will
        # invoke it from within the sandbox to create $out. The choice of
        # pure or impure mode occurs outside of this script as the derivation
        # is instantiated.
        source $stdenv/setup # is this necessary?

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
        ${ if buildCache-tar-contents == null then ":" else ''
          tar --skip-old-files -xpf ${buildCache-tar-contents}
        '' }
        ${ if buildCache == null then ''
          # When not preserving a cache we just run the build normally.
          FLOX_VIRTUAL_SANDBOX=${virtualSandbox} FLOX_SRC_DIR=$(pwd) \
          FLOX_TURBO=1 ${flox-env-package}/activate bash -e ${buildScript-contents}
        '' else ''
          # If the build fails we still want to preserve the build cache, so we
          # remove $out on failure and allow the Nix build to proceed to write
          # the result symlink.
          FLOX_VIRTUAL_SANDBOX=${virtualSandbox} FLOX_SRC_DIR=$(pwd) \
          FLOX_TURBO=1 ${flox-env-package}/activate bash -e ${buildScript-contents} || \
            ( rm -rf $out && echo "flox build failed (caching build dir)" | tee $out 1>&2 )
        '' }
      ''
    )
    + ''
      # Start by patching shebangs in bin and sbin directories.
      for dir in $out/bin $out/sbin; do
        if [ -d "$dir" ]; then
          patchShebangs $dir
        fi
      done
      # Wrap contents of files in bin with ${flox-env-package}/activate
      for prog in $out/bin/* $out/sbin/*; do
        if [ -L "$prog" ]; then
          : # You cannot wrap a symlink, so just leave it be?
        else
          assertExecutable "$prog"
          hidden="$(dirname "$prog")/.$(basename "$prog")"-wrapped
          mv "$prog" "$hidden"
          makeShellWrapper "${flox-env-package}/activate" "$prog" \
            --inherit-argv0 \
            --set FLOX_ENV "${flox-env-package}" \
            --set FLOX_TURBO 1 \
            --set FLOX_MANIFEST_BUILD_OUT "$out" \
            --set FLOX_VIRTUAL_SANDBOX "${virtualSandbox}" \
            --run 'export FLOX_SET_ARG0="$0"' \
            --add-flags "$hidden"
        fi
      done
    '' + pkgs.lib.optionalString (buildCache != null) ''
      # Only tar the files to avoid differences in directory {a,c,m}times.
      # Sort the files to keep the output stable across builds.
      # Avoid compressing with gzip because that is not stable across
      # invocations on Mac only. Experimentation shows that xz and bzip2
      # compression is stable on both Mac and Linux, but that can be slow,
      # and we probably don't actually need to compress the build cache
      # because we actively delete the old copy as we create a new one.
      find . -type f | sort | tar -c --no-recursion -f $buildCache -T -
    ''
  )
