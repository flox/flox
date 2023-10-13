{self}:
builtins.path {
  name = "flox-src";
  path = self;
  filter = path: type:
    ! builtins.elem path (map (f: self + ("/" + f)) [
      "flake.nix"
      "flake.lock"
      "pkgs"
      "checks"
      "tests"
      "shells"
      "target"
    ]);
}
