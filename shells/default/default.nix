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
    ++ [flox-pkgdb-tests flox-env-builder-tests flox-cli-tests flox-tests];

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
        flox-env-builder
        flox-cli
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
