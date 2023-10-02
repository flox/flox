{
  mkShell,
  lib,
  rustfmt,
  clippy,
  rust-analyzer,
  darwin,
  glibcLocales,
  hostPlatform,
  nix,
  cargo,
  rustc,
  rust,
  hivemind,
  cargo-watch,
  commitizen,
  rustPlatform,
  flox,
  flox-bash,
  pre-commit-check,
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
      # cargo-watch
      clippy
      rust-analyzer
      rust.packages.stable.rustPlatform.rustLibSrc
    ];
    inherit (pre-commit-check) shellHook;
  }
  // flox.envs)
