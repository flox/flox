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
  shfmt,
  mitmproxy,
  cargo-nextest,
  flox-cli,
  flox-cli-tests,
  flox-pkgdb,
  flox-manpages,
  ci ? false,
  GENERATED_DATA ? ./../../test_data/generated,
  MANUALLY_GENERATED ? ./../../test_data/manually_generated,
}: let
  # For use in GitHub Actions and local development.
  ciPackages =
    flox-pkgdb.ciPackages
    ++ flox-pkgdb.ciPackages
    ++ flox-cli.ciPackages
    ++ [
      (flox-cli-tests.override {
        PROJECT_TESTS_DIR = "/cli/tests";
        PKGDB_BIN = null;
        FLOX_BIN = null;
        KLAUS_BIN = null;
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
      mitmproxy
      yq
      cargo-nextest
      procps
    ];
in
  mkShell (
    {
      name = "flox-dev";

      inputsFrom = [
        flox-pkgdb
        (flox-cli.override {
          flox-pkgdb = null;
        })
      ];

      packages = ciPackages ++ lib.optionals (!ci) devPackages;

      shellHook =
        flox-pkgdb.devShellHook
        + flox-cli.devShellHook
        + pre-commit-check.shellHook
        + ''
          export MANPATH=${flox-manpages}/share/man:$MANPATH
        '';

      inherit GENERATED_DATA;
      inherit MANUALLY_GENERATED;
    }
    // flox-pkgdb.devEnvs
    // flox-cli.devEnvs
  )
