# Entry that forwards the namespace into a helper that does not parse. The
# error must name the helper, not this file.
{ catalogs }:
(import ./broken.nix { inherit catalogs; }).result
