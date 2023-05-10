{
  description = "Floxpkgs/Project Template";

  inputs.flox-floxpkgs.url = "github:flox/floxpkgs";
  inputs.flox-floxpkgs.inputs.flox.follows = "/";

  # Declaration of external resources
  # =================================
  inputs.shellHooks.url = "github:cachix/pre-commit-hooks.nix";
  # =================================

  outputs = args @ {flox-floxpkgs, ...}: flox-floxpkgs.project args (_: {});
}
