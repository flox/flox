{
  mkShell,
  self,
  commitizen,
}:
mkShell {
  inputsFrom = [];
  packages = [
    commitizen
    self.packages.flox.passthru.rustPlatform.rust.cargo
  ];
}
