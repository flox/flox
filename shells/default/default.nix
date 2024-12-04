{
  treefmt,
  nixfmt-rfc-style,
  commitizen,
  hivemind,
  just,
  yq,
  bashInteractive,
  lib,
  mkShell,
  procps,
  pre-commit-check,
  pstree,
  shfmt,
  mitmproxy,
  cargo-nextest,
  flox-cli,
  flox-cli-tests,
  flox-watchdog,
  flox-pkgdb,
  flox-manpages,
  flox-package-builder,
  flox-test-shells,
  stdenv,
  ci ? false,
  GENERATED_DATA ? ./../../test_data/generated,
  MANUALLY_GENERATED ? ./../../test_data/manually_generated,
}:
let
  # For use in GitHub Actions and local development.
  ciPackages =
    flox-pkgdb.ciPackages
    ++ flox-watchdog.ciPackages
    ++ flox-pkgdb.ciPackages
    ++ flox-cli.ciPackages
    ++ [
      (flox-cli-tests.override {
        PROJECT_TESTS_DIR = "/cli/tests";
        PKGDB_BIN = null;
        FLOX_BIN = null;
        WATCHDOG_BIN = null;
      })
    ];

  devPackages =
    flox-pkgdb.devPackages
    ++ flox-cli.devPackages
    ++ [
      just
      hivemind
      commitizen
      treefmt
      nixfmt-rfc-style
      shfmt
      yq
      cargo-nextest
      procps
      pstree
      bashInteractive
    ]
    ++ lib.optionals stdenv.isLinux [
      # The python3Packages.mitmproxy-macos package is broken on mac:
      #   nix-repl> legacyPackages.aarch64-darwin.python3Packages.mitmproxy-macos.meta.broken
      #   true
      # ... so only install it on Linux. It's only an optional dev dependency.
      mitmproxy
    ];
in
mkShell (
  {
    name = "flox-dev";

    inputsFrom = [
      flox-pkgdb
      (flox-cli.override {
        flox-pkgdb = null;
        flox-watchdog = null;
      })
    ];

    FLOX_SHELL_BASH = "${flox-test-shells}/bin/bash";
    FLOX_SHELL_ZSH = "${flox-test-shells}/bin/zsh";
    FLOX_SHELL_FISH = "${flox-test-shells}/bin/fish";
    FLOX_SHELL_TCSH = "${flox-test-shells}/bin/tcsh";

    packages = ciPackages ++ lib.optionals (!ci) devPackages;

    shellHook =
      flox-pkgdb.devShellHook
      + flox-watchdog.devShellHook
      + flox-cli.devShellHook
      + pre-commit-check.shellHook
      + flox-package-builder.devShellHook
      + ''
        export MANPATH=${flox-manpages}/share/man:$MANPATH
      '';

    inherit GENERATED_DATA;
    inherit MANUALLY_GENERATED;
  }
  // flox-pkgdb.devEnvs
  // flox-watchdog.devEnvs
  // flox-cli.devEnvs
)
