{
  bashNonInteractive,
  coreutils,
  daemonize,
  findutils,
  flox-activations,
  getopt,
  gnused,
  iconv,
  jq,
  ld-floxlib,
  lib,
  nawk,
  process-compose,
  runCommand,
  shellcheck,
  shfmt,
  stdenv,
  substituteAll,
  util-linuxMinimal,
}:

# We need to ensure that the flox-activations package is available.
# If it's not, we'll use the binary from the environment.
# Build or evaluate this package with `--option pure-eval false`.
assert (flox-activations == null) -> builtins.getEnv "FLOX_ACTIVATIONS_BIN" != null;
let
  # `bash-envtrace` is bashNonInteractive plus the envtrace patch, which records
  # every environment-visible variable mutation to the file named by
  # BASH_ENVTRACE_FILE. The activate script (and only that script) runs
  # under it so that attach can replay exactly the mutations performed by
  # profile.d scripts, manifest vars, and hook.on-activate.
  # bashNonInteractive is the base deliberately: the activate script is
  # always headless (no readline needed), the build is the small (~3M) one,
  # and confining the patched bash to this script keeps user-facing
  # subshells (INTERACTIVE_BASH_BIN in flox-cli) on stock bashInteractive.
  # The patch is vendored from bash-envtrace commit f9c06c5 (the repo,
  # github.com/flox/bash-envtrace, is not public, so it cannot be fetched
  # at build time); its header comment is the format specification.
  # It is in the official bash-patch format (apply with -p0), which is how
  # nixpkgs applies bash patches, and targets bash 5.3p9 sources.
  # The copy carries one local change on top of that commit, to be folded
  # back into bash-envtrace before the next re-vendoring: `prepend`/`append`
  # are reported only when the added text carries the `:` list separator
  # (see envtrace_classify_set); plain textual growth reports as `updated`.
  # Known-stale items in the vendored patch's header comment: its <op>
  # table omits `set-if-absent` and its control-variable list omits
  # BASH_ENVTRACE_RESET — both are implemented by the patch and documented
  # in cli/flox-activations/src/env_trace.rs.
  bash-envtrace =
    assert lib.assertMsg (lib.hasPrefix "5.3" bashNonInteractive.version)
      "bash-5.3-envtrace.patch targets bash 5.3 but bashNonInteractive is ${bashNonInteractive.version}; regenerate the patch (see the bash-envtrace repo's regen-patch) before bumping bash";
    bashNonInteractive.overrideAttrs (prev: {
      pname = prev.pname + "-envtrace";
      patches = (prev.patches or [ ]) ++ [ ./bash-5.3-envtrace.patch ];
    });

  # Borrowed from previous implementation of substituteAllFiles.
  substituteAllFiles =
    args:
    stdenv.mkDerivation (
      {
        name = if args ? name then args.name else baseNameOf (toString args.src);
        builder = builtins.toFile "builder.sh" ''
          set -o pipefail

          eval "$preInstall"

          args=

          pushd "$src"
          echo -ne "${builtins.concatStringsSep "\\0" args.files}" | xargs -0 -n1 -I {} -- find {} -type f -print0 | while read -d "" line; do
            mkdir -p "$out/$(dirname "$line")"
            substituteAll "$line" "$out/$line"
          done
          popd

          eval "$postInstall"
        '';
        preferLocalBuild = true;
        allowSubstitutes = false;
      }
      // args
    );

  environment-interpreter-with-paths = substituteAllFiles {
    src = ../../assets/environment-interpreter;
    files = [ "." ]; # Perform recursive substitution on all files.
    # Substitute all of the following variables.
    inherit
      coreutils
      findutils
      getopt
      jq
      nawk
      ;
    # Note that substitution doesn't work with variables containing "-"
    # so we need to create and use alternative names.
    bash_envtrace = bash-envtrace;
    process_compose = process-compose;
    # If the flox-activations package is available, use it,
    # otherwise copy the binary from the environment into the store,
    # so that sandboxed builds and flox built containers can access it.
    flox_activations = "${flox-activations}/libexec/flox-activations";
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
runCommand "flox-interpreter"
  {
    nativeBuildInputs = [ gnused ];
    outputs = [
      "out"
      "build_executable_wrapper"
    ];
  }
  ''
    # Smoke-check the tracer before assembling anything: the patched bash
    # must record an exported mutation and honor declared reset intent, or
    # every attach would fail at runtime with an empty trace.
    cat > "$TMPDIR/envtrace-smoke.sh" <<EOS
    BASH_ENVTRACE_FILE="$TMPDIR/envtrace-smoke.trace"
    export ENVTRACE_SMOKE=value
    export ENVTRACE_LIST=/a
    export ENVTRACE_LIST="\$ENVTRACE_LIST:/b"
    export ENVTRACE_LIST="/c:\$ENVTRACE_LIST"
    export ENVTRACE_DIR=/t/
    export ENVTRACE_DIR="\$ENVTRACE_DIR"nix-shell.abc
    BASH_ENVTRACE_RESET=1
    export ENVTRACE_SMOKE=value
    unset BASH_ENVTRACE_RESET BASH_ENVTRACE_FILE
    EOS
    ${bash-envtrace}/bin/bash "$TMPDIR/envtrace-smoke.sh"
    # Anchor the ops on the 0x1f field separator the Rust parser requires:
    # without it "reset" satisfies a bare "set" pattern, and a delimiter
    # change would pass the smoke test only to fail every attach at
    # runtime.
    grep -q $'\x1f'"set"$'\x1f' "$TMPDIR/envtrace-smoke.trace"
    grep -q $'\x1f'"reset"$'\x1f' "$TMPDIR/envtrace-smoke.trace"
    # List growth must be recorded as a delta carrying the separator, and
    # plain textual growth as a rewrite: attach replays deltas onto the
    # attaching shell's own value, so a nested attach — whose shell already
    # holds the grown value — would otherwise double anything that is not
    # a list (TMPDIR=/tmp/ becoming /tmp/nix-shell.abcnix-shell.abc).
    grep -q $'\x1f'"append"$'\x1f'"11"$'\x1f'"ENVTRACE_LIST"$'\x1f'":/a"$'\x1f'"::/b$" "$TMPDIR/envtrace-smoke.trace"
    grep -q $'\x1f'"prepend"$'\x1f'"11"$'\x1f'"ENVTRACE_LIST"$'\x1f'":/a:/b"$'\x1f'":/c:$" "$TMPDIR/envtrace-smoke.trace"
    grep -q $'\x1f'"updated"$'\x1f'"11"$'\x1f'"ENVTRACE_DIR"$'\x1f'":/t/"$'\x1f'":/t/nix-shell.abc$" "$TMPDIR/envtrace-smoke.trace"
    if grep -q $'\x1f'"append"$'\x1f'"11"$'\x1f'"ENVTRACE_DIR"$'\x1f' "$TMPDIR/envtrace-smoke.trace"; then
      echo "envtrace recorded textual growth of ENVTRACE_DIR as an append delta" >&2
      exit 1
    fi

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
      $out/activate.d/set-prompt.bash \
      $out/activate.d/helpers.bash \
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

    # Check the formatting of the scripts with shfmt.
    cp ${editorconfig} $build_executable_wrapper/.editorconfig
    # This will only catch extensions and shebangs that `shfmt --find` knows about.
    ${shfmt}/bin/shfmt --diff $build_executable_wrapper
    rm $build_executable_wrapper/.editorconfig
  ''
