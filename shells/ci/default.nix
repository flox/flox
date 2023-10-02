{
  mkShell,
  commitizen,
  cargo,
}:
mkShell {
  inputsFrom = [];
  packages = [
    commitizen
    cargo
  ];
}
