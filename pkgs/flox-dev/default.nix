{self}: let
  withBashDev = self.packages.flox.override {
    flox-bash = self.packages.flox-bash-dev;
  };
in
  withBashDev.overrideAttrs (prev: {
    version = let
      prefixMatch = builtins.match "([^-]+)-.*" prev.version;
    in
      if prefixMatch != null
      then builtins.head prefixMatch
      else prev.version;
  })
