# Pattern: the same helper is imported twice, forwarding a different root
# namespace each time; both contributions must be scanned.
{ catalogs, inputs }:
[
  (import ./common.nix { ns = catalogs; })
  (import ./common.nix { ns = inputs; })
]
