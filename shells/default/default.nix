{
  alejandra,
  commitizen,
  hivemind,
  ruff,
  just,
  lib,
  mkShell,
  poetry,
  python3,
  pre-commit-check,
  shfmt,
  flox-cli,
  flox-cli-tests,
  flox-pkgdb,
  flox-end2end,
  ci ? false,
}: let
  # For use in GitHub Actions and local development.
  ciPackages =
    flox-pkgdb.ciPackages
    ++ flox-pkgdb.ciPackages
    ++ flox-cli.ciPackages
    ++ [
      flox-cli-tests
      flox-end2end
      just
    ];

  devPackages =
    flox-pkgdb.devPackages
    ++ flox-cli.devPackages
    ++ [
      alejandra
      commitizen
      hivemind
      ruff
      shfmt
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
