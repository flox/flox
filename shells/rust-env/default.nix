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
  # commitizen,
}:
mkShell ({
    inputsFrom = [
      self.packages.flox
      self.packages.flox.passthru.flox-bash
    ];
    RUST_SRC_PATH = "${self.packages.flox.passthru.rustPlatform.rustLibSrc}";
    RUSTFMT = "${self.checks.pre-commit-check.passthru.rustfmt}/bin/rustfmt";
    packages = [
      # temporar until https://github.com/commitizen-tools/commitizen/pull/644 is merged
      self.checks.pre-commit-check.passthru.commitizen
      self.checks.pre-commit-check.passthru.rustfmt
      hivemind
      cargo-watch
      clippy
      rust-analyzer
      rust.packages.stable.rustPlatform.rustLibSrc
    ];
    shellHook = ''
      ${self.checks.pre-commit-check.shellHook}
    '';
  }
  // self.packages.flox.envs)
