# Pattern: the import target does not exist, so the refs it would contribute
# through the forwarded namespace cannot be discovered and the scan fails.
{ catalogs }:
import ./no-such-helper.nix { inherit catalogs; }
