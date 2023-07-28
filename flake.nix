{
  description = "Floxpkgs/Project Template";

  inputs.flox-floxpkgs.url = "github:flox/floxpkgs";
  inputs.shellHooks.url = "github:cachix/pre-commit-hooks.nix";
  inputs.crane.url = "github:ipetkov/crane";
  inputs.floco = {
    type = "github";
    owner = "aakropotkin";
    repo = "floco";
    rev = "e1231f054258f7d62652109725881767765b1efb";
    # MFB: commented 20230527, breaks the floxpkgs-internal pkgset.
    # inputs.nixpkgs.follows = "/flox-floxpkgs/nixpkgs";
  };
  inputs.parser-util.url = "github:flox/parser-util/v0";

  outputs = inputs:
    inputs.flox-floxpkgs.project inputs (_: {});
}
