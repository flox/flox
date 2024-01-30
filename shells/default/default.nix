{
  alejandra,
  commitizen,
  hivemind,
  just,
  lib,
  mkShell,
  pre-commit-check,
  shfmt,
  flox-cli,
  flox-cli-tests,
  flox-pkgdb,
  flox-tests,
  ci ? false,
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
      (flox-tests.override {
        PROJECT_TESTS_DIR = "/tests";
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
    }
    // flox-pkgdb.devEnvs
    // flox-cli.devEnvs
  )
