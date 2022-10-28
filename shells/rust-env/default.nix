{
  mkShell,
  self',
  rustfmt,
  clippy,
  rust-analyzer,
  flox,
  nix,
  rustPlatform,
}:
mkShell {
  inputsFrom = [self'.packages.flox-cli];
  packages = [rustfmt clippy rust-analyzer];
  shellHook = ''
    ${self'.checks.pre-commit-check.shellHook}
    export NIX_BIN="${nix}/bin/nix"
    export FLOX_SH="${flox}/libexec/flox/flox"
    # For use with rust-analyzer
    export RUST_SRC_PATH="${rustPlatform.rustLibSrc}"
  '';
}
