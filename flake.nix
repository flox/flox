{
  description = "Floxpkgs/Project Template";

  inputs.flox-floxpkgs.url = "github:flox/floxpkgs";
  inputs.shellHooks.url = "github:cachix/pre-commit-hooks.nix";
  inputs.crane.url = "github:ipetkov/crane";
  # Temporary while we work to fold this functionality into flox itself.
  inputs.floco = {
    type = "github";
    owner = "aakropotkin";
    repo = "floco";
    rev = "e1231f054258f7d62652109725881767765b1efb";
  };
  inputs.parser-util.url = "github:flox/parser-util/v0";
  inputs.pkgdb.url = "github:flox/pkgdb";
  inputs.sqlite3pp.url = "github:aakropotkin/sqlite3pp";

  outputs = inputs:
    inputs.flox-floxpkgs.project inputs (_: {});
}
