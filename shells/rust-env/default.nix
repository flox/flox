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
  hivemind,
  cargo-watch,
}:
mkShell ({
    inputsFrom = [
      self.packages.flox
    ];
    RUST_SRC_PATH = "${self.packages.flox.passthru.rustPlatform.rustLibSrc}";
    packages = [
      hivemind
      cargo-watch
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
