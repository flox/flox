# Pattern: the import target does not exist and the whole namespace is the
# argument; the refs it would contribute cannot be discovered and the scan
# fails.
{ catalogs }:
import ./no-such-helper.nix catalogs
