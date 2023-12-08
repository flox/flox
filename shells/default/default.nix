{
  lib,
  mkShell,
  just,
  hivemind,
  commitizen,
  alejandra,
  pre-commit-check,
  flox,
  flox-pkgdb,
  flox-env-builder,
  flox-tests,
  flox-tests-end2end,
  flox-env-builder-tests,
  flox-pkgdb-tests,
  ci ? false,
}: let
  # For use in GitHub Actions and local development.
  ciPackages =
    flox-pkgdb.ciPackages
    ++ flox-env-builder.ciPackages
    ++ flox.ciPackages
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
      (flox-tests.override {
        PROJECT_TESTS_DIR = "/tests";
        PKGDB_BIN = null;
        ENV_BUILDER_BIN = null;
        FLOX_BIN = null;
      })
      (flox-tests-end2end.override {
        PROJECT_NAME = "flox-tests-end2end";
        PROJECT_TESTS_SUBDIR = "";
        PROJECT_TESTS_DIR = "/tests/end2end";
        PKGDB_BIN = null;
        ENV_BUILDER_BIN = null;
        FLOX_BIN = null;
      })
    ];

  devPackages =
    flox-pkgdb.devPackages
    ++ flox-env-builder.devPackages
    ++ flox.devPackages
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
        flox-env-builder
        flox
      ];

      packages = ciPackages ++ lib.optionals (!ci) devPackages;

      shellHook =
        flox-pkgdb.devShellHook
        + flox-env-builder.devShellHook
        + flox.devShellHook
        + pre-commit-check.shellHook;
    }
    // flox-pkgdb.devEnvs
    // flox-env-builder.devEnvs
    // flox.devEnvs
  )
