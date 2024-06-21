{
  alejandra,
  commitizen,
  hivemind,
  just,
  yq,
  lib,
  mkShell,
  pre-commit-check,
  shfmt,
  mitmproxy,
  flox-cli,
  flox-cli-tests,
  flox-pkgdb,
  ci ? false,
  GENERATED_DATA ? ./../../test_data/generated,
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
        LD_FLOXLIB = null;
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
        + pre-commit-check.shellHook;

      inherit GENERATED_DATA;
    }
    // flox-pkgdb.devEnvs
    // flox-cli.devEnvs
  )
