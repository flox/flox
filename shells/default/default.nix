{
  alejandra,
  commitizen,
  hivemind,
  just,
  lib,
  mkShell,
  pre-commit-check,
  flox-cli,
  flox-cli-tests,
  flox-env-builder,
  flox-env-builder-tests,
  flox-pkgdb,
  flox-pkgdb-tests,
  flox-tests,
  ci ? false,
}: let
  # For use in GitHub Actions and local development.
  ciPackages =
    flox-pkgdb.ciPackages
    ++ flox-env-builder.ciPackages
    ++ flox-cli.ciPackages
    ++ [
      (flox-pkgdb-tests.override {
        PROJECT_TESTS_DIR = "/pkgdb/tests";
        PKGDB_BIN = null;
        PKGDB_IS_SQLITE3_BIN = null;
        PKGDB_SEARCH_PARAMS_BIN = null;
      })
      (flox-env-builder-tests.override {
        PROJECT_TESTS_DIR = "/env-builder/tests";
        PKGDB_BIN = null;
        ENV_BUILDER_BIN = null;
      })
      (flox-cli-tests.override {
        PROJECT_TESTS_DIR = "/cli/tests";
        PKGDB_BIN = null;
        ENV_BUILDER_BIN = null;
        FLOX_BIN = null;
      })
      (flox-tests.override {
        PROJECT_TESTS_DIR = "/tests";
        PKGDB_BIN = null;
        ENV_BUILDER_BIN = null;
        FLOX_BIN = null;
      })
    ];

  devPackages =
    flox-pkgdb.devPackages
    ++ flox-env-builder.devPackages
    ++ flox-cli.devPackages
    ++ [
      just
      hivemind
      commitizen
      alejandra
    ];
in
  mkShell (
    {
      name = "flox-dev";

      inputsFrom = [
        flox-pkgdb
        (flox-env-builder.override {flox-pkgdb = null;})
        (flox-cli.override {
          flox-pkgdb = null;
          flox-env-builder = null;
        })
      ];

      packages = ciPackages ++ lib.optionals (!ci) devPackages;

      shellHook =
        flox-pkgdb.devShellHook
        + flox-env-builder.devShellHook
        + flox-cli.devShellHook
        + pre-commit-check.shellHook;
    }
    // flox-pkgdb.devEnvs
    // flox-env-builder.devEnvs
    // flox-cli.devEnvs
  )
