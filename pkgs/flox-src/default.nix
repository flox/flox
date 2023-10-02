{self}:
builtins.path {
  name = "flox-src";
  path = self;
  filter = path: type:
    ! builtins.elem (baseNameOf path) [
      "flake.nix"
      "flake.lock"
      "pkgs"
      "checks"
      "tests"
      "shells"
    ];
}
