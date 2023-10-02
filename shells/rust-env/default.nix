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
  rustPlatform,
  cargo,
  rustc,
  rust,
  hivemind,
  cargo-watch,
  commitizen,
  just,
  pre-commit-check,
  flox,
  flox-bash,
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
      rustc
      just
    ];
    inherit (pre-commit-check) shellHook;
  }
  // flox.envs)
