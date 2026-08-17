{ }:
let
  root = ./../..;
  # Top-level entries that are not inputs to any flox-src consumer (the
  # Rust crate builds under cli/). Keeping them out means commits that
  # only touch them do not invalidate CLI builds or their caches.
  exclude = [
    "flake.nix"
    "flake.lock"
    "pkgs"
    "shells"
    "target"
    "modules"
  ];
  # The filter receives paths as strings, so the exclusions must be
  # compared as strings anchored at the source root; comparing against
  # path values (or paths resolved relative to this file's directory)
  # never matches.
  excludePaths = map (f: "${toString root}/${f}") exclude;
in
builtins.path {
  name = "flox-src";
  path = root;
  filter = path: type: !builtins.elem path excludePaths;
}
