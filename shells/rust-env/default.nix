{
  mkShell,
  self',
  rustfmt,
  clippy,
  rust-analyzer,
  flox,
  nix,
}:
mkShell {
  inputsFrom = [self'.packages.flox-cli];
  packages = [rustfmt clippy rust-analyzer];
  shellHook = ''
    ${self'.checks.pre-commit-check.shellHook}
    export NIX_BIN="${nix}/bin/nix"
    export FLOX_SH="${flox}/libexec/flox/flox"
  '';
}
