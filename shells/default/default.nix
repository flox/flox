{
  alejandra,
  commitizen,
  hivemind,
  just,
  yq,
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
  stdenv,
  ci ? false,
  GENERATED_DATA ? ./../../test_data/generated,
  MANUALLY_GENERATED ? ./../../test_data/manually_generated,
}: let
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
      alejandra
      shfmt
      yq
      cargo-nextest
      procps
      pstree
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
