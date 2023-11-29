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
      inherit (pre-commit-check) shellHook;
    }
    // flox.envs)
