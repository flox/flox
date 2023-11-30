{
  clippy,
  commitizen,
  flox,
  flox-bash,
  flox-env-builder,
  flox-tests,
  hivemind,
  just,
  mkShell,
  pre-commit-check,
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
        flox-bash
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
        flox-tests
        batsWith
      ];
      shellHook = ''
        shopt -s autocd;

        alias gs='git status';
        alias ga='git add';
        alias gc='git commit -am';
        alias gl='git pull';
        alias gp='git push';

        # For running `pkgdb' interactively with inputs from the test suite.
        NIXPKGS_TEST_REV="e8039594435c68eb4f780f3e9bf3972a7399c4b1";
        NIXPKGS_TEST_REF="github:NixOS/nixpkgs/$NIXPKGS_TEST_REV";

        # Find the project root and add the `bin' directory to `PATH'.
        if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
          PATH="$PATH:$( git rev-parse --show-toplevel; )/pkgdb/bin";
        fi

      '' + pre-commit-check.shellHook;
    }
    // flox.envs
    // flox-env-builder.envs)
