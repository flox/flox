{
  alejandra,
  commitizen,
  hivemind,
  just,
  lib,
  mkShell,
  poetry,
  python3,
  pre-commit-check,
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
      flox-cli-tests
      flox-tests
      just
    ];

  devPackages =
    flox-pkgdb.devPackages
    ++ flox-cli.devPackages
    ++ [
      alejandra
      commitizen
      hivemind
    ];
in
  mkShell (
    {
      name = "flox-dev";

      inputsFrom = [
        flox-pkgdb
        flox-cli
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
