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
  flox-tests-dev,
  flox-tests-end2end-dev,
  flox-pkgdb-tests-dev,
  ci ? false,
}: let
  # For use in GitHub Actions and local development.
  ciPackages = [
    flox-tests-dev
    flox-tests-end2end-dev
    flox-pkgdb-tests-dev
  ];

  devPackages =
    flox-pkgdb.devPackages
    ++ flox-env-builder.devPackages
    ++ flox-pkgdb.devPackages
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

      packages = ciPackages ++ lib.optionals ci devPackages;

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
