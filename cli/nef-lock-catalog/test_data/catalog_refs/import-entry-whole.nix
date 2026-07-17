# Pattern: the whole catalog namespace is the import's argument. The helper's
# lambda parameter binds the root, so its refs must be scanned and rewritten
# to the parent root.
{ catalogs }:
import ./import-helper-whole.nix catalogs
