# Pattern: the whole catalog namespace is the import's argument, but the
# helper destructures it with a pattern parameter the scanner cannot bind
# statically, so the whole root escapes analysis.
{ catalogs }:
import ./import-helper-pattern.nix catalogs
