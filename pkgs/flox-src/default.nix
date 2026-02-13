{ }:
builtins.path {
  name = "flox-src";
  path = ./../..;
  filter =
    path: type:
    !builtins.elem path (
      map (f: ./../${f}) [
        "flake.nix"
        "flake.lock"
        "pkgs"
        "checks"
        "tests"
        "shells"
        "target"
      ]
    );
}
