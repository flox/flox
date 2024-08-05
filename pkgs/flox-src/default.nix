{}:
builtins.path {
  name = "flox-src";
  path = "${./../../cli}";
  filter = path: type:
    ! builtins.elem path (map (
        f: "${./../../cli}/${f}"
      ) [
        "flake.nix"
        "flake.lock"
        "pkgs"
        "checks"
        "tests"
        "shells"
        "target"
      ]);
}
