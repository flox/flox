{
  mkShell,
  commitizen,
  cargo,
  rustc,
}:
mkShell {
  inputsFrom = [];
  packages = [
    commitizen
    cargo
    rustc
  ];
}
