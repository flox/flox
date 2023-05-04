{
  description = "Floxpkgs/Project Template";

  inputs.flox-floxpkgs.url = "github:flox/floxpkgs";
  inputs.flox-floxpkgs.inputs.flox.follows = "/";
  inputs.flox-floxpkgs.inputs.flox-bash.follows = "flox-bash";

  inputs.flox-bash.url = "github:flox/flox-bash";
  inputs.flox-bash.inputs.flox.follows = "/";
  inputs.flox-bash.inputs.flox-floxpkgs.follows = "flox-floxpkgs";

  # Declaration of external resources
  # =================================
  inputs.shellHooks.url = "github:cachix/pre-commit-hooks.nix";
  # =================================

  outputs = args @ {flox-floxpkgs, ...}: flox-floxpkgs.project args (_: {});
}
