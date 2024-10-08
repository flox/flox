{ flox }:
flox.overrideAttrs (prev: {
  version =
    let
      prefixMatch = builtins.match "([^-]+)-.*" prev.version;
    in
    if prefixMatch != null then builtins.head prefixMatch else prev.version;
})
