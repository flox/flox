{
  clippy,
  commitizen,
  flox,
  flox-bash,
  hivemind,
  just,
  mkShell,
  pre-commit-check,
  rust,
  rust-analyzer,
  rustPlatform,
  rustc,
  rustfmt,
}:
mkShell ({
    inputsFrom = [
      flox
      flox-bash
    ];
    RUST_SRC_PATH = rustPlatform.rustLibSrc.outPath;
    RUSTFMT = rustfmt.outPath + "/bin/rustfmt";
    packages = [
      commitizen
      rustfmt
      hivemind
      clippy
      rust-analyzer
      rust.packages.stable.rustPlatform.rustLibSrc
      just
    ];
    inherit (pre-commit-check) shellHook;
  }
  // flox.envs)
