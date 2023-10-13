{
  mkShell,
  self,
  rustfmt,
  clippy,
  rust-analyzer,
  rustPlatform,
  rustc,
  rust,
  hivemind,
  commitizen,
  just,
}:
mkShell ({
    inputsFrom = [
      self.packages.flox-dev
      self.packages.flox-bash-dev
    ];
    RUST_SRC_PATH = self.packages.flox.passthru.rustPlatform.rustLibSrc.outPath;
    RUSTFMT = "${self.checks.pre-commit-check.passthru.rustfmt}/bin/rustfmt";
    packages = [
      commitizen
      self.checks.pre-commit-check.passthru.rustfmt
      hivemind
      clippy
      rust-analyzer
      rust.packages.stable.rustPlatform.rustLibSrc
      rustc
      just
    ];
    inherit (self.checks.pre-commit-check) shellHook;
  }
  // self.packages.flox.envs)
