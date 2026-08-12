# Forwards a catalog rather than the whole namespace. The helper receives
# `myorg` and its refs come back rewritten under `catalogs.myorg`.
{ catalogs }:
(import ./helper.nix { myorg = catalogs.myorg; }).deep
