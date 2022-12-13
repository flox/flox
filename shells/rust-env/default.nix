{
  mkShell,
  self,
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
}:
mkShell ({
    inputsFrom = [self.packages.flox];
    RUST_SRC_PATH = "${rust.packages.stable.rustPlatform.rustLibSrc}";
    packages = [
      rustfmt
      clippy
      rust-analyzer
      rust.packages.stable.rustPlatform.rustLibSrc
    ];
    shellHook = ''
      ${self.checks.pre-commit-check.shellHook}
    '';
  }
  // self.packages.flox.envs)
