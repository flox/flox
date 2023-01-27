{
  description = "Floxpkgs/Project Template";
  nixConfig.bash-prompt = "[flox] \\[\\033[38;5;172m\\]λ \\[\\033[0m\\]";
  inputs.flox-floxpkgs.url = "github:flox/floxpkgs";
  inputs.flox-bash.url = "git+ssh://git@github.com/flox/flox-bash";

  # Declaration of external resources
  # =================================
  inputs.shellHooks.url = "github:cachix/pre-commit-hooks.nix";
  # =================================

  outputs = args @ {flox-floxpkgs, ...}: flox-floxpkgs.project args (_: {});
}
