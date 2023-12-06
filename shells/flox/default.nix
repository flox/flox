{
  clippy,
  commitizen,
  flox,
  flox-env-builder,
  flox-tests-dev,
  flox-tests-end2end-dev,
  hivemind,
  just,
  mkShell,
  rust,
  rust-analyzer,
  rustPlatform,
  rustc,
  rustfmt,
  bats,
}: let
  batsWith = bats.withLibraries (p: [
    p.bats-assert
    p.bats-file
    p.bats-support
  ]);
in
  mkShell ({
      inputsFrom = [
        flox
        flox-env-builder
      ];
      RUST_SRC_PATH = rustPlatform.rustLibSrc.outPath;
      RUSTFMT = "${rustfmt}/bin/rustfmt";
      packages = [
        commitizen
        rustfmt
        hivemind
        clippy
        rust-analyzer
        rust.packages.stable.rustPlatform.rustLibSrc
        rustc
        just
        flox-tests-dev
        flox-tests-end2end-dev
        batsWith
      ];
      shellHook =
        ''
          # Extra interactive shell settings, requires `DANK_MODE' to be set.
          if [[ "''${DANK_MODE:-0}" != 0 ]]; then
            echo "You are in 〖ｄａｎｋ ｍｏｄｅ〗" >&2;
            shopt -s autocd;

            alias gs='git status';
            alias ga='git add';
            alias gc='git commit -am';
            alias gl='git pull';
            alias gp='git push';
          fi

          # For running `pkgdb' interactively with inputs from the test suite.
          NIXPKGS_TEST_REV="e8039594435c68eb4f780f3e9bf3972a7399c4b1";
          NIXPKGS_TEST_REF="github:NixOS/nixpkgs/$NIXPKGS_TEST_REV";

          # Find the project root and add the `bin' directory to `PATH'.
          if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
            PATH="$PATH:$( git rev-parse --show-toplevel; )/pkgdb/bin";
          fi

        '';
    }
    // flox.envs
    // flox-env-builder.envs)
