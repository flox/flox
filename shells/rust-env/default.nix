{
  mkShell,
  self',
  lib,
  rustfmt,
  clippy,
  rust-analyzer,
  darwin,
  flox,
  glibcLocales,
  hostPlatform,
  nix,
  rustPlatform,
  cacert,
}:
mkShell {
  inputsFrom = [self'.packages.flox-cli];
  packages = [rustfmt clippy rust-analyzer];
  shellHook = ''
    ${self'.checks.pre-commit-check.shellHook}
    # TODO factor out to share env vars with flox-cli
    export NIX_BIN="${nix}/bin/nix"
    export FLOX_SH="${flox}/libexec/flox/flox"
    export NIXPKGS_CACERT_BUNDLE_CRT="${cacert}/etc/ssl/certs/ca-bundle.crt"
    ${lib.optionalString hostPlatform.isLinux ''
      export LOCALE_ARCHIVE="${glibcLocales}/lib/locale/locale-archive"
    ''}
    ${lib.optionalString hostPlatform.isDarwin ''
      export NIX_COREFOUNDATION_RPATH="${darwin.CF}/Library/Frameworks"
      export PATH_LOCALE="${darwin.locale}/share/locale"
    ''}
    # For use with rust-analyzer
    export RUST_SRC_PATH="${rustPlatform.rustLibSrc}"
  '';
}
