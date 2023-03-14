{
  mkShell,
  self,
  # commitizen,
}:
mkShell {
  inputsFrom = [];
  packages = [
    # temporary until https://github.com/commitizen-tools/commitizen/pull/644 is merged
    self.checks.pre-commit-check.passthru.commitizen
    self.packages.flox.passthru.rustPlatform.rust.cargo
  ];
}
