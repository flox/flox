{
  inputs,
  gnused,
  pkgsFor,
  rust-toolchain,
  rustfmt ? rust-toolchain.rustfmt,
  rust-internal-deps,
  flox-src,
}: let
  # crane (<https://crane.dev/>) library for building rust packages
  craneLib = (inputs.crane.mkLib pkgsFor).overrideToolchain rust-toolchain.toolchain;

  # build time environment variables
  envs =
    {}
    // rust-internal-deps.passthru.envs;
in
  craneLib.buildPackage ({
      pname = "klaus";
      version = envs.FLOX_VERSION;
      src = flox-src;
      cargoExtraArgs = "--locked -p klaus -p flox";

      CARGO_LOG = "cargo::core::compiler::fingerprint=info";

      cargoArtifacts = rust-internal-deps;

      postPatch = ''
        rm -rf ./flox/*
        cp -rf --no-preserve=mode ${craneLib.mkDummySrc {src = flox-src;}}/flox/* ./flox
        ls -la ./flox
      '';

      # runtime dependencies
      buildInputs = rust-internal-deps.buildInputs ++ [];

      # build dependencies
      nativeBuildInputs =
        rust-internal-deps.nativeBuildInputs
        ++ [
          gnused
        ];

      # https://github.com/ipetkov/crane/issues/385
      # doNotLinkInheritedArtifacts = true;

      # Tests are disabled inside of the build because the sandbox prevents
      # internet access and there are tests that require internet access to
      # resolve flake references among other things.
      doCheck = false;

      # bundle manpages and completion scripts
      #
      # sed: Removes rust-toolchain from binary. Likely due to toolchain overriding.
      #   unclear about the root cause, so this is a hotfix.
      postInstall = ''
        for target in "$(basename ${rust-toolchain.rust.outPath} | cut -f1 -d- )" ; do
          sed -i -e "s|$target|eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee|g" $out/bin/klaus
        done
      '';

      passthru = {
        inherit
          envs
          ;

        ciPackages = [];

        devPackages = [
          rustfmt
        ];

        devEnvs =
          envs
          // {
            RUST_SRC_PATH = "${rust-toolchain.rust-src}/lib/rustlib/src/rust/library";
            RUSTFMT = "${rustfmt}/bin/rustfmt";
          };

        devShellHook = ''
          #  # Find the project root and add the `bin' directory to `PATH'.
          if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
            PATH="$( git rev-parse --show-toplevel; )/cli/target/debug":$PATH;
            REPO_ROOT="$( git rev-parse --show-toplevel; )";
            KLAUS_BIN="$REPO_ROOT/cli/target/debug/klaus";
          fi

        '';
      };
    }
    // envs)
