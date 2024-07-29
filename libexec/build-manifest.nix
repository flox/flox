{
  pkgs ? import <nixpkgs> {},
  name,
  flox-env,
  install-prefix,
  srcdir ? null, # optional
  buildScript ? null, # optional
  buildCache ? null, # optional
}:

# First a few assertions to ensure that the inputs are consistent.

# buildCache is only meaningful with a build script
assert (buildCache != null) -> (buildScript != null);
# srcdir is only required with a build script
assert (srcdir != null) -> (buildScript != null);

let

  flox-env-package = builtins.storePath flox-env;
  install-prefix-contents = /. + install-prefix;
  src =
    if (srcdir == null)
    then null
    else builtins.fetchGit srcdir;
  buildScript-contents = /. + buildScript;
  buildCache-tgz =
    if (buildCache != null && buildCache != "") then (/. + buildCache)
    else null;

in
  pkgs.runCommand name {
    inherit src;
    buildInputs = with pkgs; [flox-env-package findutils gnutar gnused makeWrapper];
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
      ''
      else ''
        # If the build script is provided, then it's expected that we will
        # invoke it from within the sandbox to create $out. The choice of
        # pure or impure mode occurs outside of this script as the derivation
        # is instantiated.
        source $stdenv/setup
        unpackPhase
        cd "$sourceRoot"
        # Extract contents of the cache, if it exists.
        ${ if buildCache-tgz == null then ":" else
          "tar --skip-old-files -xpzf ${buildCache-tgz}" }
        ${ if buildCache == null then ''
          # When not preserving a cache we just run the build normally.
          FLOX_TURBO=1 ${flox-env-package}/activate bash -e ${buildScript-contents}
        '' else ''
          # If the build fails we still want to preserve the build cache, so we
          # remove $out on failure and allow the Nix build to proceed to write
          # the result symlink.
          FLOX_TURBO=1 ${flox-env-package}/activate bash -e ${buildScript-contents} || \
            ( rm -rf $out && echo "flox build failed (caching build dir)" | tee $out 1>&2 )
        '' }
      ''
    )
    + ''
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
            --run 'export FLOX_SET_ARG0="$0"' \
            --add-flags "$hidden"
        fi
      done
    '' + pkgs.lib.optionalString (buildCache != null) ''
      # Only tar the files to avoid differences in directory {a,c,m}times.
      # Sort the files to keep the output stable across builds.
      find . -type f | sort | tar -c -z --no-recursion -f $buildCache -T -
    ''
  )
