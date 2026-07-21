# Pattern: two intermediate helpers forward different root namespaces to the
# same shared file under identical parameter names; both composed
# contributions must be scanned.
{ catalogs, inputs }:
[
  (import ./mid1.nix { cats = catalogs; })
  (import ./mid2.nix { cats = inputs; })
]
