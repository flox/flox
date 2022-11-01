{
  mkShell,
  self',
  lib,
  rustfmt,
  clippy,
  rust-analyzer,
  darwin,
  flox,
  glibcLocales,
  hostPlatform,
  nix,
  rustPlatform,
  cargo,
  rustc,
  rust,
}:
mkShell ({
    inputsFrom = [self'.packages.flox-cli];
    RUST_SRC_PATH = "${rust.packages.stable.rustPlatform.rustLibSrc}";
    packages = [
      rustfmt
      clippy
      rust-analyzer
      rust.packages.stable.rustPlatform.rustLibSrc
    ];
    shellHook = ''
      ${self'.checks.pre-commit-check.shellHook}
    '';
  }
  // self'.packages.flox-cli.envs)
