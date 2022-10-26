{
  mkShell,
  self',
  rustfmt,
  clippy,
  rust-analyzer,
}:
mkShell {
  inputsFrom = [self'.packages.flox-cli];
  packages = [rustfmt clippy rust-analyzer];
  shellHook = ''
    ${self'.checks.pre-commit-check.shellHook}
  '';
}
