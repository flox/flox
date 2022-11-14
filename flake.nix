{
  description = "Floxpkgs/Project Template";
  nixConfig.bash-prompt = "[flox] \\[\\033[38;5;172m\\]λ \\[\\033[0m\\]";
  inputs.floxpkgs.url = "github:flox/floxpkgs";
  inputs.floxpkgs-internal.url = "git+ssh://git@github.com/flox/floxpkgs-internal?ref=flox-cli";

  # Declaration of external resources
  # =================================
  inputs.shellHooks = {
    url = "github:cachix/pre-commit-hooks.nix";
    inputs.nixpkgs.follows = "floxpkgs/nixpkgs/nixpkgs";
  };
  # =================================

  outputs = args @ {floxpkgs, ...}: floxpkgs.project args (_: {});
}
