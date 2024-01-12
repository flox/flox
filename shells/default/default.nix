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
    ++ [flox-cli-tests flox-tests];

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
        flox-cli
      ];

      packages = ciPackages ++ lib.optionals (!ci) devPackages;

      shellHook =
        flox-pkgdb.devShellHook
        + flox-cli.devShellHook
        + lib.optionalString (!ci) pre-commit-check.shellHook;
    }
    // flox-pkgdb.devEnvs
    // flox-cli.devEnvs
  )
